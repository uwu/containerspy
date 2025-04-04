# ContainerSpy

ContainerSpy is a lightweight daemon that connects to Docker, collects metrics (CPU%, RAM, etc.), and outputs those
via OpenTelemetry OTLP.

You can then send that to a metrics store such as Prometheus, a collection agent such as Grafana Alloy,
or a cloud observability platform.

Note that containerspy currently targets only Docker, not Kubernetes or any other orchestration systems.
It outputs the same traces as cAdvisor for drop-in compatibility with existing data series and dashboards.

## Why make this?

ContainerSpy is intended to replace [cAdvisor](https://github.com/google/cadvisor) in a Prometheus/Grafana monitoring
setup. It takes inspiration from [Beszel](https://www.beszel.dev/) in its approach.
The main reason for this to exist is my personal difficulties deploying cAdvisor.

cAdvisor is rather RAM-heavy, and it really does not need to be so.
It also requires a plethora of different mounts to get working inside of a container, including /sys, or even the entire
/ filesystem, and in some cases must be ran as a privileged user!
Even with this, it can often just completely fail to collect CPU usage data depending on your distro.

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
