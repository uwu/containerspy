use std::mem::MaybeUninit;
use bollard::container::{BlkioStatsEntry, MemoryStatsStats, MemoryStatsStatsV1, StatsOptions};
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

		// use the first read only for stats diffing for blkio - don't need for cpu thanks to precpu.
		#[allow(unused_assignments)]
		let mut first_read = MaybeUninit::uninit();

		loop {
			match stats_stream.next().await {
				None => return,
				Some(Ok(st)) => { first_read = MaybeUninit::new(st); break },
				Some(Err(err)) => {
					// TODO: use json logging or syslog so loki can understand this lol
					println!("Failed to get stats for container {container_id}!: {err:?}");
				}
			}
		}

		// I'm going to rust jail!
		let first_read = unsafe { first_read.assume_init() };
		let mut last_io_stats = first_read.blkio_stats.io_service_bytes_recursive;

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
			.with_unit("By")
			.with_description("Cumulative bytes read")
			.build();
		let meter_container_fs_writes_bytes_total = meter
			.u64_counter("container_fs_writes_bytes_total")
			.with_unit("By")
			.with_description("Cumulative bytes written")
			.build();

		let meter_container_last_seen = meter
			.u64_gauge("container_last_seen")
			.with_description("Last time this container was seen by ContainerSpy")
			.build();

		while let Some(val) = stats_stream.next().await {
			if let Ok(stats) = val {

				// when a container exits, instead of a None we get sent Ok()s with zeroes in it forever, horror
				if stats.cpu_stats.cpu_usage.total_usage == 0 {
					if stats.precpu_stats.cpu_usage.total_usage != 0 { break; }
					else {
						// last time was ALSO a zero, so this MIGHT actually be (SOMEHOW?) legit,
						// so just loop around again, and wait for the main task to abort() this worker task instead!
						// which it will if this container died, or if we are gonna get real stats later, it won't...
						// man i dont know i should probably just break lol
						continue;
					}
				};

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
					// need to calculate deltas for this
					if let Some(last) = &last_io_stats {

						let (last_r, last_w) = get_rw_totals(last);
						let (curr_r, curr_w) = get_rw_totals(&service_bytes_rec);

						meter_container_fs_reads_bytes_total.add(curr_r - last_r, shared_labels);
						meter_container_fs_writes_bytes_total.add(curr_w - last_w, shared_labels);
					}

					last_io_stats = Some(service_bytes_rec);
				}

				meter_container_last_seen.record(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(), shared_labels);

				// cgroups values references:
				// - https://github.com/docker/cli/blob/91cbde67/cli/command/container/stats_helpers.go#L230-L231
				// - https://github.com/google/cadvisor/blob/f6e31a3c/info/v1/container.go#L389 (yes, v1, roll w it)
				// - https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html

				if let Some(all_usage) = stats.memory_stats.usage {
						if cfg!(windows) {
							// todo
							// i have no way to test cgroups v2 so only work on v1 - see readme for more info
						} else if let Some(MemoryStatsStats::V2(v2stats)) = stats.memory_stats.stats {
							// container_memory_cache


							// container_memory_failcnt only on cgroups v1

							// container_memory_failures_total
							v2stats.pgfault; // label failure_type=pgfault
							v2stats.pgmajfault; // label failure_type=pgmajfault

							// container_memory_mapped_file
							v2stats.file; // includes tmpfs

							// container_memory_max_usage_bytes only on cgroups v1

							// container_memory_migrate


							// container_memory_numa_pages omitted cause its hard :<

							// container_memory_rss: may need recalcing

							// container_memory_swap: can't get

							// container_memory_usage_bytes: how?
							
							// container_memory_working_set_bytes: not reported
						}
					}
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

fn get_rw_totals<'a>(iter: impl IntoIterator<Item = &'a BlkioStatsEntry>) -> (u64, u64) {
	let mut read = 0;
	let mut write = 0;

	for entry in iter {
		match entry.op.as_str() {
			"read" => read += entry.value,
			"write" => write += entry.value,
			_ => println!("Unknown service_bytes_recursive entry type {}", entry.op)
		}
	}

	(read, write)
}