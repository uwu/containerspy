use std::{collections::BTreeMap, time::Duration};

use anyhow::Result;
use bollard::{container::StatsOptions, Docker};
use config::CONFIG;
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry::metrics::MeterProvider;
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
						println!("{e}");
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
	let docker =
		if let Some(path) = &CONFIG.docker_socket {
			Docker::connect_with_socket(path, 60, bollard::API_DEFAULT_VERSION)?
		}
		else {
			Docker::connect_with_local_defaults()?
		};

	// connect the OTLP exporter
	let meter_provider = setup_otlp()?;

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

		let list_res = docker.list_containers::<String>(None).await?;

		let container_ids: Vec<_> = list_res.into_iter().filter_map(|c| c.id).collect();
		container_ids.sort();

		let mut to_remove = Vec::new();

		for (cont, handle) in &tasks {
			// funny O(n^2) loop
			if container_ids.binary_search(cont).is_err() {
				handle.abort();
				to_remove.push(cont);
			}
		}

		for cont in to_remove {
			tasks.remove(cont);
		}

		// now, add any new ones
		for cont in &container_ids {
			if !tasks.contains_key(cont) {
				tasks.insert(cont.clone(), launch_stats_task());
			}
		}
	}

	/*  let list_res = docker.list_containers::<String>(None).await?;

		let cont_name = list_res[0].id.as_ref().unwrap().as_str();

		// df takes a moment so also select on it
		let mut df =
			/* tokio::select! {
				df = docker.df() => { df }
				_ = shutdown_token.cancelled() => { break }
			}; */
			docker.stats(cont_name/* .trim_start_matches("/") */, Some(StatsOptions {
				stream: true,
				one_shot: false
			}));

		// drop the first one
		df.next().await;

		while let Some(v) = df.next().await {
			let v = v?;
			println!("{v:?}");
		}
 */
	Ok(())
}

fn launch_stats_task<'a>(container_id: &str, docker: &Docker, meter_provider: &impl MeterProvider) -> JoinHandle<()> {
	tokio::spawn(async move {
		let mut stats_stream =
			docker.stats(container_id, Some(StatsOptions {
				stream: true,
				one_shot: false
			}));

		// drop the first one
		stats_stream.next().await;

		while let Some(val) = stats_stream.next().await {
			if let Ok(stats) = val {

			}
			else {
				// failed to get stats, log as such:
				// TODO: use json logging or syslog so loki can understand this lol
				println!("Failed to get stats for container {container_id}!: {:?}", val.unwrap_err());
			}
		}
	})
}
