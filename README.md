# ContainerSpy

ContainerSpy is a lightweight daemon that connects to Docker, collects metrics (CPU%, RAM, etc.), and outputs those
via OpenTelemetry OTLP.

You can then send that to a metrics store such as Prometheus, a collection agent such as Grafana Alloy,
or a cloud observability platform.

Note that containerspy currently targets only Docker, not Kubernetes or any other orchestration systems.
It outputs the same traces as cAdvisor for drop-in compatibility with existing data series and dashboards.

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

## How to set up

TODO: will write once it actually works

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

Note: to send directly to Prometheus (with `--enable-feature=otlp-write-receiver`), use
http://localhost:9090/api/v1/otlp/v1/metrics as your endpoint, swapping `localhost:9090` for your Prometheus `host:port`.

## Supported metrics

This is intended to be a dropin replacement for cAdvisor, which lists its supported metrics
[here](https://github.com/google/cadvisor/blob/master/docs/storage/prometheus.md).

All generic labels attached to all metrics are implemented, and the status of labels applied only to specific metrics
is listed below ("N/A" if there are none).

The list of ContainerSpy's currently supported items from this list is:

| Name                                        | Has metric-specific labels | Notes                   |
|---------------------------------------------|----------------------------|-------------------------|
| `container_cpu_usage_seconds_total`         |                            |                         |
| `container_cpu_user_seconds_total`          |                            |                         |
| `container_cpu_system_seconds_total`        |                            |                         |
| `container_cpu_cfs_periods_total`           |                            |                         |
| `container_cpu_cfs_throttled_periods_total` |                            |                         |
| `container_cpu_cfs_throttled_seconds_total` |                            |                         |
| `container_fs_reads_bytes_total`            |                            | Not reported on Windows |
| `container_fs_writes_bytes_total`           |                            | Not reported on Windows |
| `container_last_seen`                       |                            |                         |

The list of known omitted metrics are:

| Name                                             | Reason                            |
|--------------------------------------------------|-----------------------------------|
| `container_cpu_load_average_10s`                 | Not reported by Docker Engine API |
| `container_cpu_schedstat_run_periods_total`      | Not reported by Docker Engine API |
| `container_cpu_schedstat_runqueue_seconds_total` | Not reported by Docker Engine API |
| `container_cpu_schedstat_run_seconds_total`      | Not reported by Docker Engine API |
| `container_file_descriptors`                     | Not reported by Docker Engine API |
| `container_fs_inodes_free`                       | Not reported by Docker Engine API |
| `container_fs_inodes_total`                      | Not reported by Docker Engine API |
| `container_fs_io_current`                        | Not reported by Docker Engine API |
| `container_fs_io_time_seconds_total`             | Only reported on cgroups v1 hosts |
| `container_fs_io_time_weighted_seconds_total`    | Not reported by Docker Engine API |
| `container_fs_limit_bytes`                       | Not reported by Docker Engine API |
| `container_fs_read_seconds_total`                | Only reported on cgroups v1 hosts |
| `container_fs_reads_merged_total`                | Only reported on cgroups v1 hosts |
| `container_fs_reads_total`                       | Not reported by Docker Engine API |
| `container_fs_sector_reads_total`                | Only reported on cgroups v1 hosts |
| `container_fs_write_seconds_total`               | Only reported on cgroups v1 hosts |
| `container_fs_writes_merged_total`               | Only reported on cgroups v1 hosts |
| `container_fs_writes_total`                      | Not reported by Docker Engine API |
| `container_fs_sector_writes_total`               | Only reported on cgroups v1 hosts |
| `container_fs_usage_bytes`                       | Requires SystemDataUsage API      |
| `container_hugetlb_failcnt`                      | Not reported by Docker Engine API |
| `container_hugetlb_max_usage_bytes`              | Not reported by Docker Engine API |
| `container_hugetlb_usage_bytes`                  | Not reported by Docker Engine API |
