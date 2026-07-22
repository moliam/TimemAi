# TimemAi

TimemAi is a local-first AI agent with two interfaces:

- `timem`: terminal UI for shell-heavy work.
- `timem-web`: browser UI for sessions, chat history, tools, and live work status.

Both interfaces use the same local runtime, memory, session history, tools, and
provider configuration.

## Install

```bash
git clone https://github.com/moliam/TimemAi.git
cd TimemAi
./install.sh
```

The installer builds and installs `timem` and `timem-web`. Rust dependencies
are downloaded automatically. The released Web bundle is already included;
Node.js is only needed when developing the Web frontend.

## Configure

Create a private environment file:

```bash
cp env_template env
$EDITOR env
source ./env
```

Minimum Aliyun-compatible configuration:

```bash
export TIMEM_GATEWAY_PROVIDER=aliyun
export TIMEM_API_KEY=your_api_key_here
export TIMEM_MODEL=qwen-plus
export TIMEM_SPACE=.test_mem
```

The environment file can be stored elsewhere:

```bash
source /path/to/your/env
```

Use `timem --help` or `timem-web --help` to inspect available startup
options. Command-line options override environment variables.

## Run Shell

```bash
source ./env
timem
```

Common controls:

- `/help`: show interactive commands.
- `/config`: change runtime settings.
- `/workspace`: manage the working directory.
- `/prof`: inspect runtime profiling.
- `Ctrl+C` or `Esc`: cancel the current input, menu, or thinking turn.
- `Ctrl+D` or `/exit`: exit the shell.

While the model is working, type an additional instruction and press Enter to
send it as a supplement to the current task.

## Run Web

Local mode binds to `127.0.0.1` and opens the authenticated page automatically
when a local graphical session is available:

```bash
source ./env
timem-web
```

On SSH or headless Linux, the server prints the URL without trying to open a
browser. Open that URL on a machine with a browser.

Public mode binds to all interfaces and prints a token-protected URL:

```bash
source ./env
timem-web --public
```

Open the complete URL printed in the terminal, including `?token=...`, from
your local browser. The port is selected automatically. To choose a fixed
port or advertised host:

```bash
timem-web --public --port 20699 --public-host 10.125.112.83
```

Public mode does not open a browser on the server. HTTP access may show a
browser "Not secure" warning because it uses plain HTTP; the access token is
still required. For production exposure, place Timem behind HTTPS and an
appropriate network access control layer.

## More Documentation

- [Architecture](docs/architecture.md)
- [Install and configuration](docs/install-and-configuration.md)
- [Core/UI topic protocol](docs/core-ui-topic-protocol.md)
- [Capability system](docs/capability-system.md)
- [Test strategy](docs/test-strategy.md)
- [Release smoke checklist](docs/manual-release-smoke.md)

## Update and Uninstall

```bash
git pull --ff-only
./install.sh
```

```bash
./uninstall.sh
```

Runtime data and private environment files are user-managed and are not
removed by uninstall.

Please star [moliam/TimemAi](https://github.com/moliam/TimemAi).
