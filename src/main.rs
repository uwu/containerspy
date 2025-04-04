use std::time::Duration;

use anyhow::Result;
use bollard::Docker;
use config::CONFIG;
use opentelemetry::{metrics::MeterProvider, KeyValue};
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::{metrics::SdkMeterProvider, resource::{ResourceDetector, SdkProvidedResourceDetector}, Resource};
use tokio_util::sync::CancellationToken;

mod config;

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

	let info = docker.info().await?;

	println!("Connected to Docker Daemon version {:?}", info.server_version);

	// connect the OTLP exporter
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

	let meter_provider = SdkMeterProvider::builder()
		.with_periodic_exporter(metric_exporter)
		.build();

	// fetch-report loop with graceful shutdown
	let shutdown_token = CancellationToken::new();
	let st2 = shutdown_token.clone(); // to be moved into the task

	tokio::spawn(async move {
		tokio::signal::ctrl_c().await.expect("Failed to setup ctrl-c handler");
		st2.cancel();
	});

	let mut interval = tokio::time::interval(Duration::from_secs(1));

	loop {
		tokio::select! {
			_ = interval.tick() => {}
			_ = shutdown_token.cancelled() => { break }
		}

		let list_res = docker.list_containers::<String>(None).await?;
		println!("{list_res:?}");
	}

	Ok(())
}
