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

| `config.json`   | env var              | description                                                       | default    |
| --------------- | -------------------- | ----------------------------------------------------------------- | ---------- |
| `docker_socket` | `CSPY_DOCKER_SOCKET` | The docker socket / named pipe to connect to                      | unset      |
| `otlp_protocol` | `CSPY_OTLP_PROTO`    | Whether to use httpbinary, httpjson, or grpc to send OTLP metrics | httpbinary |

You can set configuration in the config file specified in the `CSPY_CONFIG` env variable
(`/etc/containerspy/config.json`) by default, which supports JSON5 syntax, or configure via the `CSPY_` env vars.

If a docker socket path is not set, containerspy will try to connect to
`/var/run/docker.sock` or `//./pipe/docker_engine` depending on host OS.

## Supported metrics

This is intended to be a dropin replacement for cAdvisor, which lists its supported metrics
[here](https://github.com/google/cadvisor/blob/master/docs/storage/prometheus.md).

The list of ContainerSpy's currently supported items from this list is:
 - `container_cpu_usage_seconds_total`
