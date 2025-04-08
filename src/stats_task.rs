use bollard::container::{BlkioStatsEntry, MemoryStatsStats, Stats, StatsOptions};
use bollard::models::ContainerSummary;
use bollard::Docker;
use opentelemetry::metrics::MeterProvider;
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use std::mem::MaybeUninit;
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
		// TODO: this is acceptable for specifically the use case of the michiru deployment, but not more generally at all
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
				Some(Ok(st)) => {
					first_read = MaybeUninit::new(st);
					break;
				}
				Some(Err(err)) => {
					// TODO: use json logging or syslog so loki can understand this lol
					println!("Failed to get stats for container {container_id}!: {err:?}");
				}
			}
		}

		// I'm going to rust jail!
		let first_read = unsafe { first_read.assume_init() };
		let Stats {
			blkio_stats,
			networks: mut last_net_stats,
			memory_stats: mut last_mem_stats,
			..
		} = first_read;

		let mut last_io_stats = blkio_stats.io_service_bytes_recursive;

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

		// other label sets that are static per container
		let mut labels_mem_container_min_c = shared_labels.clone();
		labels_mem_container_min_c.push(KeyValue::new("failure_type", "pgfault"));

		let mut labels_mem_container_maj_c = shared_labels.clone();
		labels_mem_container_maj_c.push(KeyValue::new("failure_type", "pgmajfault"));

		let mut labels_mem_container_min_h = labels_mem_container_min_c.clone();
		labels_mem_container_min_h.push(KeyValue::new("scope", "hierarchy"));
		labels_mem_container_min_c.push(KeyValue::new("scope", "container"));

		let mut labels_mem_container_maj_h = labels_mem_container_maj_c.clone();
		labels_mem_container_maj_h.push(KeyValue::new("scope", "hierarchy"));
		labels_mem_container_maj_c.push(KeyValue::new("scope", "container"));

		// free space and make immutable
		shared_labels.shrink_to_fit();
		let shared_labels = &shared_labels[..];

		labels_mem_container_min_c.shrink_to_fit();
		labels_mem_container_min_h.shrink_to_fit();
		labels_mem_container_maj_c.shrink_to_fit();
		labels_mem_container_maj_h.shrink_to_fit();
		let labels_mem_container_min_c = &labels_mem_container_min_c[..];
		let labels_mem_container_min_h = &labels_mem_container_min_h[..];
		let labels_mem_container_maj_c = &labels_mem_container_maj_c[..];
		let labels_mem_container_maj_h = &labels_mem_container_maj_h[..];

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

		// annoyingly a lot of the meter names cadvisor went with don't have units attached even though they have known units
		let meter_container_memory_cache = meter
			.u64_gauge("container_memory_cache")
			//.with_unit("By")
			.with_description("Total page cache memory")
			.build();
		let meter_container_memory_failures_total = meter
			.u64_counter("container_memory_failures_total")
			.with_description("Cumulative count of memory allocation failures")
			.build();
		let meter_container_memory_mapped_file = meter
			.u64_gauge("container_memory_mapped_file")
			//.with_unit("By")
			.with_description("Size of memory mapped files")
			.build();
		let meter_container_memory_rss = meter
			.u64_gauge("container_memory_rss")
			//.with_unit("By")
			.with_description("Size of RSS")
			.build();
		let meter_container_memory_usage_bytes = meter
			.u64_gauge("container_memory_usage_bytes")
			.with_unit("By")
			.with_description(
				"Current memory usage, including all memory regardless of when it was accessed",
			)
			.build();
		let meter_container_memory_working_set_bytes = meter
			.u64_gauge("container_memory_working_set_bytes")
			.with_unit("By")
			.with_description("Current working set")
			.build();

		let meter_container_network_receive_bytes_total = meter
			.u64_counter("container_network_receive_bytes_total")
			.with_unit("By")
			.with_description("Cumulative count of bytes received")
			.build();
		#[cfg(not(windows))]
		let meter_container_network_receive_errors_total = meter
			.u64_counter("container_network_receive_errors_total")
			.with_description("Cumulative count of errors encountered while receiving")
			.build();
		let meter_container_network_receive_packets_dropped_total = meter
			.u64_counter("container_network_receive_packets_dropped_total")
			.with_description("Cumulative count of packets dropped while receiving")
			.build();
		let meter_container_network_receive_packets_total = meter
			.u64_counter("container_network_receive_packets_total")
			.with_description("Cumulative count of packets received")
			.build();

		let meter_container_network_transmit_bytes_total = meter
			.u64_counter("container_network_transmit_bytes_total")
			.with_unit("By")
			.with_description("Cumulative count of bytes transmitted")
			.build();
		#[cfg(not(windows))]
		let meter_container_network_transmit_errors_total = meter
			.u64_counter("container_network_transmit_errors_total")
			.with_description("Cumulative count of errors encountered while transmitting")
			.build();
		let meter_container_network_transmit_packets_dropped_total = meter
			.u64_counter("container_network_transmit_packets_dropped_total")
			.with_description("Cumulative count of packets dropped while transmitting")
			.build();
		let meter_container_network_transmit_packets_total = meter
			.u64_counter("container_network_transmit_packets_total")
			.with_description("Cumulative count of packets transmitted")
			.build();

		let meter_container_start_time_seconds = meter
			.u64_gauge("container_start_time_seconds")
			.with_unit("s")
			.with_description("Start time of the container since unix epoch")
			.build();

		let meter_container_threads = meter
			.u64_gauge("container_threads")
			.with_description("Number of threads running inside the container")
			.build();
		let meter_container_threads_max = meter
			.u64_gauge("container_threads_max")
			.with_description("Maximum number of threads allowed inside the container")
			.build();

		while let Some(val) = stats_stream.next().await {
			if let Ok(stats) = val {
				// when a container exits, instead of a None we get sent Ok()s with zeroes in it forever, horror
				if stats.cpu_stats.cpu_usage.total_usage == 0 {
					if stats.precpu_stats.cpu_usage.total_usage != 0 {
						break;
					} else {
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
					cpu_delta_from_docker(
						stats.cpu_stats.throttling_data.throttled_time,
						stats.precpu_stats.throttling_data.throttled_time,
					)
					.as_secs_f64(),
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
				// TODO: handle windows storage stats

				meter_container_last_seen.record(
					SystemTime::now()
						.duration_since(UNIX_EPOCH)
						.unwrap()
						.as_secs(),
					shared_labels,
				);

				// cgroups values references:
				// - https://github.com/docker/cli/blob/91cbde67/cli/command/container/stats_helpers.go#L230-L231
				// - https://github.com/google/cadvisor/blob/f6e31a3c/info/v1/container.go#L389 (yes, v1, roll w it)
				// - https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html

				// see https://stackoverflow.com/a/66778814 and also https://archive.is/qJWTp
				// also see this comparison between cAdvisor output and {stats.memory_stats.usage:?} {v2stats:?}
				// on my dev laptop: https://web.archive.org/web/20250408121954/https://pastebin.com/Kc4Ur0Hr
				// and jackpot: https://github.com/google/cadvisor/blob/1f17a6c/container/libcontainer/handler.go#L808

				if let Some(all_usage) = stats.memory_stats.usage {
					if cfg!(windows) {
						// todo
						// i have no way to test cgroups v2 so only work on v1 - see readme for more info
					} else if let Some(MemoryStatsStats::V2(v2stats)) = stats.memory_stats.stats {
						// container_memory_cache
						meter_container_memory_cache.record(v2stats.file, shared_labels);

						// container_memory_failures_total
						// need last
						if let Some(MemoryStatsStats::V2(last_v2)) = last_mem_stats.stats {
							meter_container_memory_failures_total.add(
								v2stats.pgfault - last_v2.pgfault,
								labels_mem_container_min_c,
							);
							meter_container_memory_failures_total.add(
								v2stats.pgfault - last_v2.pgfault,
								labels_mem_container_min_h,
							);

							meter_container_memory_failures_total.add(
								v2stats.pgmajfault - last_v2.pgmajfault,
								labels_mem_container_maj_c,
							);
							meter_container_memory_failures_total.add(
								v2stats.pgmajfault - last_v2.pgmajfault,
								labels_mem_container_maj_h,
							);
						}

						// container_memory_kernel_usage
						// actually not reported by cA but is reported by docker!
						// not sure if slab contains kernel_stack or not though
						// in my one sample, kernel_stack < slab
						//v2stats.slab + v2stats.kernel_stack;

						// container_memory_mapped_file
						meter_container_memory_mapped_file.record(v2stats.file_mapped, shared_labels); // includes tmpfs

						// container_memory_rss
						meter_container_memory_rss.record(v2stats.anon, shared_labels);

						// container_memory_swap: can't get
						// need accesss to memory.swap.*, but we only have memory.stat :(

						// container_memory_usage_bytes
						meter_container_memory_usage_bytes.record(all_usage, shared_labels);

						// container_memory_working_set_bytes
						meter_container_memory_working_set_bytes
							.record(all_usage - v2stats.inactive_file, shared_labels);
					}
				}

				last_mem_stats = stats.memory_stats;

				// networking
				// TODO: what is stats.network? is it populated on windows?
				if let Some(net) = &stats.networks {
					if let Some(last_net_stats) = &last_net_stats {
						for (interface, this_inter) in net {
							// try to get last
							if let Some(last_this_inter) = last_net_stats.get(interface) {
								// net labels
								let mut net_labels = Vec::with_capacity(shared_labels.len() + 1);
								net_labels.extend_from_slice(shared_labels);
								net_labels.push(KeyValue::new("interface", interface.clone()));
								let net_labels = &net_labels.into_boxed_slice()[..];

								meter_container_network_receive_bytes_total
									.add(this_inter.rx_bytes - last_this_inter.rx_bytes, net_labels);
								meter_container_network_transmit_bytes_total
									.add(this_inter.tx_bytes - last_this_inter.tx_bytes, net_labels);
								#[cfg(not(windows))]
								meter_container_network_receive_errors_total
									.add(this_inter.rx_errors - last_this_inter.rx_errors, net_labels);
								#[cfg(not(windows))]
								meter_container_network_transmit_errors_total
									.add(this_inter.tx_errors - last_this_inter.tx_errors, net_labels);
								meter_container_network_receive_packets_dropped_total.add(
									this_inter.rx_dropped - last_this_inter.rx_dropped,
									net_labels,
								);
								meter_container_network_transmit_packets_dropped_total.add(
									this_inter.tx_dropped - last_this_inter.tx_dropped,
									net_labels,
								);
								meter_container_network_receive_packets_total.add(
									this_inter.rx_packets - last_this_inter.rx_packets,
									net_labels,
								);
								meter_container_network_transmit_packets_total.add(
									this_inter.tx_packets - last_this_inter.tx_packets,
									net_labels,
								);
							}
						}
					}
				}
				last_net_stats = stats.networks;

				if let Some(pid_count) = stats.pids_stats.current {
					// pid_count is generally the *thread* count.
					meter_container_threads.record(pid_count, shared_labels);
				}
				if let Some(pid_limit) = stats.pids_stats.limit {
					// pid_count is generally the *thread* count.
					meter_container_threads_max.record(pid_limit, shared_labels);
				}

				if let Some(Ok(secs)) = container.created.map(u64::try_from) {
					//let date = UNIX_EPOCH + Duration::from_nanos(secs);
					meter_container_start_time_seconds.record(secs, shared_labels);
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
			_ => println!("Unknown service_bytes_recursive entry type {}", entry.op),
		}
	}

	(read, write)
}

// LMAO i built this entire string pool around the idea of needing &'static str but turns out i can just use owned strings
// guuuh okay whatever that's fine i guess, i'll keep this around just in case i need it -- sink

/*
// labels have to have 'static values so i have to make a string pool or i'll leak ram, eugh
// technically this does mean each possible kv combo is never dropped, but we only have one copy in ram at all times
// Arc would also work, but that would require a lot of refcounting for a count we know will NEVER hit zero
// so just use a Cow that borrows a leaked box instead.
// I checked, OtelString can either be owned (from String), borrowed (from Cow<'static, str>), or refcounted (Arc<str>).

static LABEL_POOL: LazyLock<RwLock<HashMap<(Cow<str>, Cow<str>), KeyValue>>> = LazyLock::new(|| RwLock::new(HashMap::new()));
fn pool_kv(key: &str, val: &str) -> KeyValue {
	let leaked_k = &*Box::leak(key.to_string().into_boxed_str());
	let leaked_v = &*Box::leak(val.to_string().into_boxed_str());

	let cows = (Cow::from(leaked_k), Cow::from(leaked_v));

	if let Some(kv) = LABEL_POOL.read().unwrap().get(&cows) {
		// this should borrow the same value thanks to OtelString::Borrowed :)
		kv.clone()
	} else {
		// we know upfront that the cow is borrowed, so just clone it
		let kv = KeyValue::new(cows.0.clone(), cows.1.clone());
		LABEL_POOL.write().unwrap().insert(cows, kv.clone());
		kv
	}
}*/
