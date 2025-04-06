use bollard::container::StatsOptions;
use bollard::models::ContainerSummary;
use bollard::Docker;
use opentelemetry::metrics::MeterProvider;
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

// I do not enjoy taking a bunch of Rcs but tokio needs ownership so fine.
pub fn launch_stats_task(
	container: ContainerSummary,
	docker: Arc<Docker>,
	meter_provider: Arc<SdkMeterProvider>,
) -> JoinHandle<()> {
	tokio::spawn(async move {
		// extract some container info
		let container_id = container.id.unwrap();
		let container_name = container
			.names
			.iter()
			.flatten()
			.next()
			.map(|n| n.trim_start_matches("/").to_owned());

		let meter_name = "cspy_".to_string() + container_id.as_str();
		// lol 'static moment
		let meter_name = &*Box::leak(meter_name.into_boxed_str());

		let mut stats_stream = docker.stats(
			container_id.as_str(),
			Some(StatsOptions {
				stream: true,
				one_shot: false,
			}),
		);

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
			KeyValue::new(
				"image",
				container.image.unwrap_or(container.image_id.unwrap()),
			),
		];

		if let Some(name) = container_name {
			shared_labels.push(KeyValue::new("name", name));
		}

		if let Some(docker_labels) = &container.labels {
			for (key, value) in docker_labels {
				shared_labels.push(KeyValue::new(
					"container_label_".to_string() + key,
					value.clone(),
				))
			}
		}

		// free space and make mutable
		shared_labels.shrink_to_fit();
		let shared_labels = &shared_labels[..];

		//println!("Starting reporting for container: {shared_labels:?}");

		// create meters
		let meter = meter_provider.meter(meter_name);

		let meter_container_cpu_usage_seconds_total = meter
			.f64_counter("container_cpu_usage_seconds_total")
			.with_unit("s")
			.with_description("Cumulative cpu time consumed")
			.build();
		let meter_container_cpu_user_seconds_total = meter
			.f64_counter("container_cpu_user_seconds_total")
			.with_unit("s")
			.with_description("Cumulative userland cpu time consumed")
			.build();
		let meter_container_cpu_system_seconds_total = meter
			.f64_counter("container_cpu_system_seconds_total")
			.with_unit("s")
			.with_description("Cumulative kernel cpu time consumed")
			.build();

		let meter_container_cpu_cfs_periods_total = meter
			.u64_counter("container_cpu_cfs_periods_total")
			.with_description("Number of elapsed enforcement period intervals")
			.build();
		let meter_container_cpu_cfs_throttled_periods_total = meter
			.u64_counter("container_cpu_cfs_throttled_periods_total")
			.with_description("Number of throttled period intervals")
			.build();
		let meter_container_cpu_cfs_throttled_seconds_total = meter
			.f64_counter("container_cpu_cfs_throttled_seconds_total")
			.with_unit("s")
			.with_description("Total time duration the container has been throttled")
			.build();

		let meter_container_fs_reads_bytes_total = meter
			.u64_counter("container_fs_reads_bytes_total")
			.with_unit("B")
			.with_description("Cumulative bytes read")
			.build();
		let meter_container_fs_writes_bytes_total = meter
			.u64_counter("container_fs_writes_bytes_total")
			.with_unit("B")
			.with_description("Cumulative bytes written")
			.build();

		let meter_container_last_seen = meter
			.u64_gauge("container_last_seen")
			.with_unit("s")
			.with_description("Last time this container was seen by ContainerSpy")
			.build();

		while let Some(val) = stats_stream.next().await {
			if let Ok(stats) = val {
				meter_container_cpu_usage_seconds_total.add(
					cpu_delta_from_docker(
						stats.cpu_stats.cpu_usage.total_usage,
						stats.precpu_stats.cpu_usage.total_usage,
					)
					.as_secs_f64(),
					shared_labels,
				);

				meter_container_cpu_user_seconds_total.add(
					cpu_delta_from_docker(
						stats.cpu_stats.cpu_usage.usage_in_usermode,
						stats.precpu_stats.cpu_usage.usage_in_usermode,
					)
					.as_secs_f64(),
					shared_labels,
				);

				meter_container_cpu_system_seconds_total.add(
					cpu_delta_from_docker(
						stats.cpu_stats.cpu_usage.usage_in_kernelmode,
						stats.precpu_stats.cpu_usage.usage_in_kernelmode,
					)
					.as_secs_f64(),
					shared_labels,
				);

				meter_container_cpu_cfs_periods_total.add(
					stats.cpu_stats.throttling_data.periods - stats.precpu_stats.throttling_data.periods,
					shared_labels,
				);

				meter_container_cpu_cfs_throttled_periods_total.add(
					stats.cpu_stats.throttling_data.throttled_periods
						- stats.precpu_stats.throttling_data.throttled_periods,
					shared_labels,
				);

				meter_container_cpu_cfs_throttled_seconds_total.add(
					cpu_delta_from_docker(stats.cpu_stats.throttling_data.throttled_time,
						 stats.precpu_stats.throttling_data.throttled_time).as_secs_f64(),
					shared_labels,
				);

				// other blkio_stats values only exist on cgroups v1 so don't bother.
				// io_service_bytes_recursive exists only on cgroups v1.
				// storage_stats only exists on windows.
				if let Some(service_bytes_rec) = stats.blkio_stats.io_service_bytes_recursive {
					println!("{service_bytes_rec:?}"); // todo: remove

					for entry in &service_bytes_rec {
						match entry.op.as_str() {
							"read" =>
								meter_container_fs_reads_bytes_total.add(entry.value, shared_labels),
							"write" =>
								meter_container_fs_writes_bytes_total.add(entry.value, shared_labels),
							_ => println!("Unknown service_bytes_recursive entry type {}", entry.op)
						}
					}
				}

				meter_container_last_seen.record(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(), shared_labels);
			} else {
				// failed to get stats, log as such:
				// TODO: use json logging or syslog so loki can understand this lol
				println!(
					"Failed to get stats for container {container_id}!: {:?}",
					val.unwrap_err()
				);
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
