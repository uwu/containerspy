use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::Result;
use bollard::{container::StatsOptions, Docker};
use bollard::models::ContainerSummary;
use opentelemetry::KeyValue;
use config::CONFIG;
use opentelemetry::metrics::MeterProvider;
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

mod config;

fn setup_otlp() -> Result<SdkMeterProvider> {
	let metric_exporter =
		match CONFIG.otlp_protocol {
			Protocol::HttpBinary | Protocol::HttpJson => {
				let builder = MetricExporter::builder().with_http().with_protocol(CONFIG.otlp_protocol);
				let builder =
					if let Some(e) = &CONFIG.otlp_endpoint {
						builder.with_endpoint(e)
					} else {
						builder
					};

				builder.build()?
			},
			Protocol::Grpc => {
				let builder = MetricExporter::builder().with_tonic().with_protocol(Protocol::Grpc);

				let builder =
					if let Some(e) = &CONFIG.otlp_endpoint {
						builder.with_endpoint(e.as_str())
					} else {
						builder
					};

				builder.build()?
			},
		};

	Ok(SdkMeterProvider::builder()
			.with_periodic_exporter(metric_exporter)
			.build())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
	// open a docker connection
	let docker = Arc::new(
		if let Some(path) = &CONFIG.docker_socket {
			Docker::connect_with_socket(path, 60, bollard::API_DEFAULT_VERSION)?
		}
		else {
			Docker::connect_with_local_defaults()?
		}
	);

	// connect the OTLP exporter
	let meter_provider = Arc::new(setup_otlp()?);

	// fetch-report loop with graceful shutdown
	let shutdown_token = CancellationToken::new();
	let st2 = shutdown_token.clone(); // to be moved into the task

	tokio::spawn(async move {
		tokio::signal::ctrl_c().await.expect("Failed to setup ctrl-c handler");
		st2.cancel();
	});

	let mut container_search_interval = tokio::time::interval(Duration::from_secs(1));

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
			if containers.binary_search_by(|c| c.id.as_ref().unwrap().cmp(cont)).is_err() {
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
				// all this string cloning hurts me
				tasks.insert(id_string.clone(), launch_stats_task(cont, docker.clone(), meter_provider.clone()));
			}
		}
	}

	// abort all stats tasks
	for task in tasks.into_values() {
		task.abort();
	}

	println!("clean shutdown.");

	Ok(())
}

// I do not enjoy taking a bunch of Rcs but tokio needs ownership so fine.
fn launch_stats_task(container: ContainerSummary, docker: Arc<Docker>, meter_provider: Arc<SdkMeterProvider>) -> JoinHandle<()> {
	tokio::spawn(async move {
		// extract some container info
		let container_id = container.id.unwrap();
		let container_name = container.names.iter().flatten().next().map(|n| n.trim_start_matches("/").to_owned());

		let mut stats_stream =
			docker.stats(container_id.as_str(), Some(StatsOptions {
				stream: true,
				one_shot: false
			}));

		// drop the first read
		loop {
			match stats_stream.next().await {
				None => return,
				Some(Ok(_)) => break,
				Some(Err(err)) => {
					// TODO: use json logging or syslog so loki can understand this lol
					println!("Failed to get stats for container {container_id}!: {err:?}");
				}
			}
		}

		// container labels shared for all metrics
		let mut shared_labels = vec![
			KeyValue::new("id", container_id.to_owned()),
			KeyValue::new("image", container.image.unwrap_or(container.image_id.unwrap()))
		];

		if let Some(name) = container_name {
			shared_labels.push(KeyValue::new("name", name));
		}

		if let Some(docker_labels) = &container.labels {
			for (key, value) in docker_labels {
				shared_labels.push(KeyValue::new("container_label_".to_string() + key, value.clone()))
			}
		}

		// free space and make mutable
		shared_labels.shrink_to_fit();
		let shared_labels = &shared_labels[..];

		//println!("Starting reporting for container: {shared_labels:?}");

		// create meters
		let meter_container_cpu_usage_seconds_total = meter_provider.meter("test_meter").f64_counter("container_cpu_usage_seconds_total").with_unit("s").with_description("Cumulative cpu time consumed").build();

		while let Some(val) = stats_stream.next().await {
			if let Ok(stats) = val {
				meter_container_cpu_usage_seconds_total.add(
					cpu_delta_from_docker(stats.cpu_stats.cpu_usage.total_usage, stats.precpu_stats.cpu_usage.total_usage).as_secs_f64(),
					shared_labels);
			}
			else {
				// failed to get stats, log as such:
				// TODO: use json logging or syslog so loki can understand this lol
				println!("Failed to get stats for container {container_id}!: {:?}", val.unwrap_err());
			}
		}
	})
}

fn cpu_delta_from_docker(cpu_usage: u64, precpu_usage: u64) -> Duration {
	let delta = cpu_usage - precpu_usage;

	// https://docs.docker.com/reference/api/engine/version/v1.48/#tag/Container/operation/ContainerStats
	// see response schema > cpu_stats > cpu_usage > total_usage
	let delta_ns = if cfg!(windows) { delta * 100 } else { delta };

	Duration::from_nanos(delta_ns)
}