# Local OpenHands Agent-Server Pin

This directory pins the local OpenHands agent-server package used by the OpenSymphony daemon in supervised local mode.

- Pinned release: `v1.14.0`
- Source of truth: [`version.txt`](./version.txt)
- Python package: `openhands-agent-server==1.14.0`

The version above matches the latest GitHub release that was visible from `https://github.com/OpenHands/software-agent-sdk/releases` on March 21, 2026.

## Usage

1. Install [`uv`](https://docs.astral.sh/uv/).
2. Run `uv sync` in this directory to create a local virtual environment.
3. Start the server with `./run-local.sh`.

The Rust daemon will eventually supervise this command directly. For M1, this directory exists to make the pin and local operator workflow explicit and reviewable.
