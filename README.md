# ContainerSpy

ContainerSpy is a lightweight daemon that connects to Docker, collects metrics (CPU%, RAM, etc.), and outputs those
via OpenTelemetry OTLP.

You can then send that to a metrics store such as Prometheus, a collection agent such as Grafana Alloy,
or a cloud observability platform.

Note that containerspy currently targets only Docker, not Kubernetes or any other orchestration systems.
It outputs the same traces as cAdvisor for drop-in compatibility with existing data series and dashboards.

README CONTENTS:
 - [Why make/use this](#why-makeuse-this)
 - [Install Instructions](#install-instructions)
 - [How to Configure](#how-to-configure)
 - [Exporting to Prometheus](#exporting-to-prometheus)
 - [Exporting to Grafana Alloy](#exporting-to-grafana-alloy)
 - [TODO](#todo)
 - [Supported Metrics](#supported-metrics)

## Why make/use this?

ContainerSpy is intended to replace [cAdvisor](https://github.com/google/cadvisor) in a Prometheus/Grafana monitoring
setup. It takes inspiration from [Beszel](https://www.beszel.dev/) in its approach.
The main reason for this to exist is my personal difficulties deploying cAdvisor.

cAdvisor is rather RAM-heavy, and it really does not need to be so.
It also requires a plethora of different mounts to get working inside of a container, including /sys, or even the entire
/ filesystem, and in some cases must be ran as a privileged user!

This is mostly because cAdvisor actually collects statistics on *cgroups*, not specifically on docker containers.
It does have specific integration for containerd, docker, and podman, but it will also happily report statistics about
systemd services to Prometheus too!
If you only want to support Docker, you need not bother with cgroups, as the Docker Engine can report all you need.

I have previously used Beszel for my monitoring, and it's agent runs as an unprivileged user,
needs access to only the docker socket, collects all data out of the box, and has a lightweight footprint.

ContainerSpy aims to do what beszel-agent does, but instead of outputting to an opinionated AIO system,
outputs to (e.g.) Prometheus for a more heavyweight setup.

**I can highly recommend Beszel** as an easy to setup monitoring solution for Docker.
It will give you CPU use, RAM use, disk and bandwidth use, swap use, both system-wide and per-container, with
configurable email alerting OOTB with very little setup. It is a great piece of software.

My motivation to move to a Prometheus/Grafana setup is that I want the centralised rich logging that Loki can give me.

## Install Instructions

See the following section for detailed instructions on configuring containerspy.

<details>
<summary>Running the binary directly</summary>

If you are running an instance of the [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/)
on localhost, then you can simply
```bash
./containerspy
```

To pass configuration options you can either (recommended for quick testing) use env vars:
```bash
CSPY_OLTP_PROTO=grpc ./containerspy
```

or you can create a config file. If you create it in `/etc/containerspy/config.json`, it will be picked up
automatically, and you can just run `./containerspy` as before. If you create it anywhere else, you can specify its
location as so:
```bash
CSPY_CONFIG=./config.json ./containerspy
```
</details>

<details>
<summary>Docker</summary>

You can use either env vars or a config.json file to configure containerspy:

```bash
docker run \
	-v /var/run/docker.sock:/var/run/docker.sock:ro \
	-v ./config.json:/etc/containerspy/config.json:ro \
	ghcr.io/uwu/containerspy
```

```bash
docker run \
	-v /var/run/docker.sock:/var/run/docker.sock:ro \
	-e CSPY_XXX=YYY \ # see the configuring instructions below
	ghcr.io/uwu/containerspy
```
</details>

<details>
<summary>Docker Compose</summary>

```yml
services:
	containerspy:
		image: ghcr.io/uwu/containerspy
		volumes:
			- /var/run/docker.sock:/var/run/docker.sock:ro
		environment:
			CSPY_OTLP_ENDPOINT: http://collector:4318
#			CSPY_OTLP_INTERVAL: 30000 # 30s
		networks: [otlpnet]

# OTLP collector (you can use any OTLP receiver such as Alloy, Mimir, Prometheus)
#	collector:
#		image: otel/opentelemetry-collector-contrib
#		networks: [otlpnet]
#		...

networks:
	otlpnet:
```
</details>

It is also possible to run containerspy as a service for your preferred init system, if that's your preference.
No service files / units are provided here, please research your init system.

## How to configure

| `config.json`          | env var              | description                                                       | default                                              |
|------------------------|----------------------|-------------------------------------------------------------------|------------------------------------------------------|
| `docker_socket`        | `CSPY_DOCKER_SOCKET` | The docker socket / named pipe to connect to                      | default docker socket for host OS                    |
| `otlp_protocol`        | `CSPY_OTLP_PROTO`    | Whether to use httpbinary, httpjson, or grpc to send OTLP metrics | httpbinary                                           |
| `otlp_endpoint`        | `CSPY_OTLP_ENDPOINT` | Where to post metrics to                                          | OTLP spec default endpoint                           |
| `otlp_export_interval` | `CSPY_OTLP_INTERVAL` | How often to report metrics, in milliseconds                      | value of `OTEL_METRIC_EXPORT_INTERVAL` or 60 seconds |

You can set configuration in the config file specified in the `CSPY_CONFIG` env variable
(`/etc/containerspy/config.json` by default), which supports JSON5 syntax, or configure via the `CSPY_` env vars.

If a docker socket path is not set, ContainerSpy will try to connect to
`/var/run/docker.sock` on *NIX or `//./pipe/docker_engine` on Windows.

If an endpoint is not set, CSpy will try to post to the default ports and endpoints for an OTLP collector running on
the chosen protocol (`http://localhost:4318` for HTTP, `http://localhost:4317` for gRPC, see
[here](https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/protocol/exporter.md) and
[here](https://github.com/open-telemetry/opentelemetry-rust/blob/bc82d4f6/opentelemetry-otlp/src/exporter/mod.rs#L60)).

## Exporting to [Prometheus](https://prometheus.io/)

First, enable Prometheus' OTLP write receiver by starting it with the `--enable-feature=otlp-write-receiver` flag.
If you are using docker compose, you would add this like so:

```yml
prometheus:
	image: prom/promtheus
	# ...
	command: --enable-feature=otlp-write-receiver
```

Then find the host for your instance, this is likely to be `localhost:9090`, or for a containerised setup,
the name of your container, eg `prometheus:9090`, and configure the ContainerSpy OTLP endpoint with this host, as
`http://host/api/v1/otlp/v1/metrics`.

A full example compose file for containerspy and prometheus is:

```yml
services:
	containerspy:
		image: ghcr.io/uwu/containerspy
		volumes:
			- /var/run/docker.sock:/var/run/docker.sock:ro
		environment:
			CSPY_OTLP_ENDPOINT: http://prometheus:9090/api/v1/otlp/v1/metrics
		networks: [otlpnet]

	prometheus:
		image: prom/prometheus
		volumes:
			- ./prometheus:/prometheus
		# ports not necessary for cspy to send metrics in this example,
		# but necessary for you to access the prom dashboard
		ports: ['9090:9090']
		command: --enable-feature=otlp-write-
		networks: [otlpnet]

networks:
	otlpnet:
```

## Exporting to [Grafana Alloy](https://grafana.com/docs/alloy/latest/)

Sending your metrics to Alloy allows you to perform extra filtering and processing, and centralise your collection.

In your config.alloy file, if you don't already have an `otelcol.receiver.otlp` block setup, create one:

You can use either http or grpc. I will use grpc here but http works just fine, both binary and json.

```
otelcol.receiver.otlp "container_metrics" {
	grpc {
	}

	output {
		metrics = []
	}
}
```

Now route containerspy's output to this: assuming for simplicity that alloy is running on localhost,
just `otlp_protocol: "grpc"` will do it, but if its somewhere else, you'll need that and
`otlp_endpoint: "http://alloy-host:4317"`, or whatever it happens to be.

Naturally if you're using HTTP then use the appropriate settings
(default protocol and endpoint is `http://host:4318/v1/metrics`)

Then you can place the name of another node in the metrics array to do whatever processing you may want on the metrics.
For example, you could add an `otelcol.processor.batch` node and set
`metrics = [otelcol.processor.batch.container_metrics.input]` to group metrics into larger batches before submitting
to the next nodes for better compression and performance (if you are using cspy at much larger scales and delayed
metrics are acceptable this could be useful).

You could also use an `otelcol.processor.filter` block to apply OTTL (OpenTelemetry Transformation Language) statements.

Or for a simple setup you could just route it into an instance of Prometheus, Mimir, Splunk, Kafka, S3, DataDog, or
even back out as OTLP to pass to another node, whatever you need!:

```
// receive metrics from containerspy
otelcol.receiver.otlp "container_metrics" {
	grpc {
	}

	output {
		metrics = [otelcol.exporter.prometheus.container_metrics.input]
	}
}

// convert the metrics from OTLP format to Prometheus format (any exporter of your choosing will work, naturally)
otelcol.exporter.prometheus "container_metrics" {
	forward_to = [prometheus.remote_write.default.receiver]
}

// send converted metrics to the Prometheus server, be it a self-hosted Prometheus, Mimir, Grafana Cloud, etc.
prometheus.remote_write "default" {
	endpoint {
		url = "http://..."
	}
}
```

## TODO

ContainerSpy is now ready for deployment, but is WIP. The planned features are:
 - implement cpu and fs metric labels
 - implement any metrics that should be available on Windows but aren't
 - automatically load configs from ./config.json too
 - (maybe?) add `--config` as another way to specify the location of the config file
 - (maybe?) read swap metrics if /sys is mounted (technically out of scope but might add anyway, not sure...)

## Supported metrics

!!! CONTAINERSPY DOES NOT SUPPORT CGROUPS V1 !!!
*Most* RAM metrics will be unavailable on cgoups v1 and any v1-only metrics are excluded.
ContainerSpy only officially supports Windows and Linux on cgroups v2. It will, however, not break on cgroups v1 hosts
and should just have missing metrics.
Yes, I know that implementing RAM metrics for cgroups is totally possible, and in fact more data is available in many
cases, but I have no system to test on, and you really should be using v2 by now.

This is intended to be a dropin replacement for cAdvisor, which lists its supported metrics
[here](https://github.com/google/cadvisor/blob/master/docs/storage/prometheus.md).

All generic labels attached to all metrics are implemented, and the status of labels applied only to specific metrics
is listed below ("N/A" if there are none).

The list of ContainerSpy's currently supported items from this list is:

| Name                                               | Metric-specific labels  | Notes                          |
|----------------------------------------------------|-------------------------|--------------------------------|
| `container_cpu_usage_seconds_total`                | TODO: `cpu`             |                                |
| `container_cpu_user_seconds_total`                 | N/A                     |                                |
| `container_cpu_system_seconds_total`               | N/A                     |                                |
| `container_cpu_cfs_periods_total`                  |                         |                                |
| `container_cpu_cfs_throttled_periods_total`        |                         |                                |
| `container_cpu_cfs_throttled_seconds_total`        |                         |                                |
| `container_fs_reads_bytes_total`                   | TODO: `device`          | Not reported on Windows (TODO) |
| `container_fs_writes_bytes_total`                  | TODO: `device`          | Not reported on Windows (TODO) |
| `container_last_seen`                              | N/A                     |                                |
| `container_memory_cache`                           | N/A                     | Not reported on Windows        |
| `container_memory_failures_total`                  | `failure_type`, `scope` | Not reported on Windows        |
| `container_memory_mapped_file`                     | N/A                     | Not reported on Windows        |
| `container_memory_rss`                             | N/A                     | Not reported on Windows        |
| `container_memory_usage_bytes`                     | N/A                     | Not reported on Windows        |
| `container_memory_working_set_bytes`               | N/A                     | Not reported on Windows        |
| `container_network_receive_bytes_total`            | `interface`             |                                |
| `container_network_receive_errors_total`           | `interface`             | Not reported on Windows        |
| `container_network_receive_packets_dropped_total`  | `interface`             |                                |
| `container_network_receive_packets_total`          | `interface`             |                                |
| `container_network_transmit_bytes_total`           | `interface`             |                                |
| `container_network_transmit_errors_total`          | `interface`             | Not reported on Windows        |
| `container_network_transmit_packets_dropped_total` | `interface`             |                                |
| `container_network_transmit_packets_total`         | `interface`             |                                |
| `container_start_time_seconds`                     | N/A                     |                                |

Additional TODO: figure out which of these metrics are or are not reportable on Windows.

The list of known omitted metrics are:

| Name                                             | Reason                                                      |
|--------------------------------------------------|-------------------------------------------------------------|
| `container_cpu_load_average_10s`                 | Not reported by Docker Engine API                           |
| `container_cpu_schedstat_run_periods_total`      | Not reported by Docker Engine API                           |
| `container_cpu_schedstat_runqueue_seconds_total` | Not reported by Docker Engine API                           |
| `container_cpu_schedstat_run_seconds_total`      | Not reported by Docker Engine API                           |
| `container_file_descriptors`                     | Not reported by Docker Engine API                           |
| `container_fs_inodes_free`                       | Not reported by Docker Engine API                           |
| `container_fs_inodes_total`                      | Not reported by Docker Engine API                           |
| `container_fs_io_current`                        | Not reported by Docker Engine API                           |
| `container_fs_io_time_seconds_total`             | Only reported on cgroups v1 hosts                           |
| `container_fs_io_time_weighted_seconds_total`    | Not reported by Docker Engine API                           |
| `container_fs_limit_bytes`                       | Not reported by Docker Engine API                           |
| `container_fs_read_seconds_total`                | Only reported on cgroups v1 hosts                           |
| `container_fs_reads_merged_total`                | Only reported on cgroups v1 hosts                           |
| `container_fs_reads_total`                       | Not reported by Docker Engine API                           |
| `container_fs_sector_reads_total`                | Only reported on cgroups v1 hosts                           |
| `container_fs_write_seconds_total`               | Only reported on cgroups v1 hosts                           |
| `container_fs_writes_merged_total`               | Only reported on cgroups v1 hosts                           |
| `container_fs_writes_total`                      | Not reported by Docker Engine API                           |
| `container_fs_sector_writes_total`               | Only reported on cgroups v1 hosts                           |
| `container_fs_usage_bytes`                       | Requires SystemDataUsage API                                |
| `container_hugetlb_failcnt`                      | Not reported by Docker Engine API                           |
| `container_hugetlb_max_usage_bytes`              | Not reported by Docker Engine API                           |
| `container_hugetlb_usage_bytes`                  | Not reported by Docker Engine API                           |
| `container_llc_occupancy_bytes`                  | Not reported by Docker Engine API                           |
| `container_memory_bandwidth_bytes`               | Not reported by Docker Engine API                           |
| `container_memory_bandwidth_local_bytes`         | Not reported by Docker Engine API                           |
| `container_memory_failcnt`                       | Only reported on cgroups v1 hosts                           |
| `container_memory_kernel_usage`                  | Undocumented, cspy has it, but i'm unsure my math's right!  |
| `container_memory_max_usage_bytes`               | Only reported on cgroups v1 hosts                           |
| `container_memory_migrate`                       | Not reported by Docker Engine API (or cA on my pc!)         |
| `container_memory_numa_pages`                    | Difficult to collect, not reported by cA on my pc           |
| `container_memory_swap`                          | Not reported by Docker Engine API                           |
| `container_network_advance_tcp_stats_total`      | Not reported by Docker Engine API                           |
| `container_network_tcp6_usage_total`             | Not reported by Docker Engine API                           |
| `container_network_tcp_usage_total`              | Not reported by Docker Engine API                           |
| `container_network_udp6_usage_total`             | Not reported by Docker Engine API                           |
| `container_network_udp_usage_total`              | Not reported by Docker Engine API                           |
| `container_oom_events_total`                     | Not reported by Docker Engine API                           |
| `container_perf_*`, `container_uncore_perf_*`    | Not reported by Docker Engine API                           |
| `container_processes`                            | Not reported by Docker Engine API (only threads, not procs) |
| `container_referenced_bytes`                     | Collection affects paging and causes mem latency            |
| `container_sockets`                              | Not reported by Docker Engine API                           |
| `container_spec_*`                               | Not reported by Docker Engine API                           |
| `container_tasks_state`                          | Not reported by Docker Engine API                           |
| `container_ulimits_soft`                         | Not reported by Docker Engine API                           |
| `machine_*`                                      | Out of scope, liable to be incorrect when containerised     |
