use anyhow::Result;
use bollard::Docker;
use config::CONFIG;
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use std::env::args;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use opentelemetry::metrics::MeterProvider;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use crate::s_log::*;

mod config;
mod stats_task;
mod s_log;

// includes data from Cargo.toml and other sources using the `built` crate
pub mod built_info {
	include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

fn setup_otlp() -> Result<SdkMeterProvider> {
	let metric_exporter = match CONFIG.otlp_protocol {
		Protocol::HttpBinary | Protocol::HttpJson => {
			let builder = MetricExporter::builder()
				.with_http()
				.with_protocol(CONFIG.otlp_protocol);
			let builder = if let Some(e) = &CONFIG.otlp_endpoint {
				builder.with_endpoint(e)
			} else {
				builder
			};

			builder.build()?
		}
		Protocol::Grpc => {
			let builder = MetricExporter::builder()
				.with_tonic()
				.with_protocol(Protocol::Grpc);

			let builder = if let Some(e) = &CONFIG.otlp_endpoint {
				builder.with_endpoint(e.as_str())
			} else {
				builder
			};

			builder.build()?
		}
	};

	// if we have a CSPY_OTLP_INTERVAL, apply that,
	// else use default behaviour which reads OTEL_METRIC_EXPORT_INTERVAL else uses one minute as the interval
	// note that a PeriodicReader without setting .with_interval is equivalent to using .with_periodic_exporter

	let reader_builder = PeriodicReader::builder(metric_exporter);
	let reader_builder = if let Some(interval) = CONFIG.otlp_export_interval {
		reader_builder.with_interval(Duration::from_millis(interval))
	} else {
		reader_builder
	};

	Ok(SdkMeterProvider::builder()
		.with_reader(reader_builder.build())
		.build())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
	// handle CLI stuff
	for arg in args() {
		if ["--version", "--help"].contains(&arg.as_str()) {
			println!(
				"ContainerSpy v{}, made with love by {}",
				built_info::PKG_VERSION,
				built_info::PKG_AUTHORS.replace(":", ", ")
			);

			if arg == "--help" {
				println!(
					"\n{}",
					include_str!("help.txt")
						.trim_end()
						.replace("{{REPO_URL}}", built_info::PKG_REPOSITORY)
				);
			}

			return Ok(());
		}
	}

	// open a docker connection
	let docker = Arc::new(if let Some(path) = &CONFIG.docker_socket {
		Docker::connect_with_socket(path, 60, bollard::API_DEFAULT_VERSION)?
	} else {
		Docker::connect_with_local_defaults()?
	});

	// connect the OTLP exporter
	let meter_provider = Arc::new(setup_otlp()?);
	let meter = Arc::new(meter_provider.meter("cspy_worker"));

	// fetch-report loop with graceful shutdown
	let shutdown_token = CancellationToken::new();
	let st2 = shutdown_token.clone(); // to be moved into the task

	tokio::spawn(async move {
		if tokio::signal::ctrl_c().await.is_ok() {
			st2.cancel();
		} else {
			warn("Failed to setup SIGINT handler, metrics may be dropped on exit", []);
		}
	});

	let mut container_search_interval =
		tokio::time::interval(Duration::from_millis(CONFIG.otlp_export_interval.unwrap_or(6000)) / 2);
	container_search_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

	let mut tasks: BTreeMap<String, JoinHandle<()>> = BTreeMap::new();

	loop {
		tokio::select! {
			_ = container_search_interval.tick() => {}
			_ = shutdown_token.cancelled() => { break }
		}

		let containers = docker.list_containers::<String>(None).await?;
		let mut containers: Vec<_> = containers.into_iter().filter(|c| c.id.is_some()).collect();

		containers.sort_by(|a, b| a.id.as_ref().unwrap().cmp(b.id.as_ref().unwrap()));

		let mut to_remove = Vec::new();

		for (cont, handle) in &tasks {
			// funny O(n^2) loop
			if containers
				.binary_search_by(|c| c.id.as_ref().unwrap().cmp(cont))
				.is_err()
			{
				debug(format_args!("Killing worker for {}", cont), [("container_id", &**cont)]);
				handle.abort();
				to_remove.push(cont.clone());
			}
		}

		for cont in to_remove.into_iter() {
			tasks.remove(&cont);
		}

		// now, add any new ones
		for cont in containers {
			let id_string = cont.id.as_ref().unwrap();
			if !tasks.contains_key(id_string) {
				debug(format_args!("Launching worker for {}", id_string), [("container_id", &**id_string)]);
				// all this string cloning hurts me
				tasks.insert(
					id_string.clone(),
					stats_task::launch_stats_task(cont, docker.clone(), meter.clone()),
				);
			}
		}
	}

	// abort all stats tasks
	for task in tasks.into_values() {
		task.abort();
	}

	debug("Exiting cleanly", []);

	let _ = meter_provider.force_flush();

	Ok(())
}
