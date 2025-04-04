use std::time::Duration;

use anyhow::Result;
use bollard::Docker;
use config::CONFIG;
use opentelemetry::{metrics::MeterProvider, KeyValue};
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::{metrics::SdkMeterProvider, Resource};

mod config;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
	/* // open a docker connection
	let docker =
		if let Some(path) = &CONFIG.docker_socket {
			Docker::connect_with_socket(path, 60, bollard::API_DEFAULT_VERSION)?
		}
		else {
			Docker::connect_with_local_defaults()?
		};

	let info = docker.info().await?;

	println!("Connected to Docker Daemon version {:?}", info.server_version); */

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

	//let test_resource = Resource::builder().with_service_name("containerspy").build();

	let meter_provider = SdkMeterProvider::builder()
	//.with_resource(test_resource)
		//.with_periodic_exporter(opentelemetry_stdout::MetricExporter::default())
		.with_periodic_exporter(metric_exporter)
		.build();

	let m = meter_provider
		.meter("test_meter")
		.u64_gauge("testing_gauge")
		.build();

	m.record(10, &[KeyValue::new("label", 4)]);


	Ok(())
}
