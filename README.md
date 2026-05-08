# Zenoh Hammer

A graphical desktop tool for testing [Zenoh](https://zenoh.io/) pub/sub and query-based distributed systems. Provides the same functionality as the `z_sub`, `z_put`, and `z_get` command-line tools, but with an interactive UI, live data viewers, and saveable session configurations.

[中文/Chinese README](https://github.com/sanri/zenoh-hammer/blob/main/README.zh.md)

---

## Screenshots

![](media/example1.png)
![](media/example2.png)
![](media/example3.png)
![](media/example4.png)
![](media/example5.png)

---

## Features

- **Session page** — load and manage Zenoh session config files; displays own ZenohID, connected peers, and connected routers once a session is open
- **Subscribe page** — declare multiple subscribers with configurable key expressions, QoS parameters, and locality; live message viewer with frequency display
- **Put page** — publish payloads with configurable encoding, priority, congestion control, and attachments
- **Get page** — send queries with custom parameters, timeout, consolidation mode, and an optional payload
- **Multi-format data viewer** — automatic detection and rendering of text, JSON (interactive tree), images (PNG, JPEG, GIF, BMP, WebP), and binary (hex dump)
- **Saveable configurations** — the full app state (all subscribers, puts, gets, and session file paths) can be saved and restored as a JSON file
- Supports Zenoh **1.9.0**

---

## Installation

Pre-built binaries for Linux, Windows, and macOS are available on the [Releases](https://github.com/sanri/zenoh-hammer/releases) page.

---

## Building from source

**Linux** — install system dependencies first:

```shell
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

Then build:

```shell
# Standard release build
cargo build --release --package zenoh-hammer

# Release build with LTO (smaller binary, slower compile)
cargo build --profile release-lto --package zenoh-hammer
```

The binary is placed in `target/release/zenoh-hammer` (or `target/release-lto/zenoh-hammer`).

---

## Session configuration

Zenoh Hammer connects to a Zenoh network using a **session config file** — a TOML or JSON5 file that tells Zenoh how and where to connect. You must create at least one before opening a session.

### Adding a config file to the app

1. Go to the **Session** page.
2. Click the **`+`** button in the left panel.
3. Select your config file (`.toml`, `.json5`, or `.json`).
4. Click **load** to preview the file contents, then **open session** to connect.

The app shows your own ZenohID, connected peers, and connected routers once the session is open.

### Config file format

Config files can be written in **TOML** (simpler syntax) or **JSON5** (supports comments).

---

### Common configurations

#### Peer mode — local network discovery (zero configuration)

Peer mode uses multicast to automatically discover other Zenoh nodes on the same network segment. No router needed.

**TOML:**
```toml
mode = "peer"
```

**JSON5:**
```json5
{
  mode: "peer"
}
```

---

#### Client mode — connect to a local router

Use this when a `zenohd` router is running on the same machine (default port 7447).

**TOML:**
```toml
mode = "client"
connect = { endpoints = ["tcp/127.0.0.1:7447"] }
```

**JSON5:**
```json5
{
  mode: "client",
  connect: { endpoints: ["tcp/127.0.0.1:7447"] }
}
```

---

#### Client mode — connect to a remote router

Replace the IP and port with your router's address.

**TOML:**
```toml
mode = "client"
connect = { endpoints = ["tcp/192.168.1.100:7447"] }
```

**JSON5:**
```json5
{
  mode: "client",
  connect: { endpoints: ["tcp/192.168.1.100:7447"] }
}
```

---

#### Client mode — with shared memory (same machine, high throughput)

Shared memory avoids copying large payloads when publisher and subscriber are on the same machine.

**TOML:**
```toml
mode = "client"
connect = { endpoints = ["tcp/127.0.0.1:7447"] }
transport = { shared_memory = { enabled = true } }
```

**JSON5:**
```json5
{
  mode: "client",
  connect: { endpoints: ["tcp/127.0.0.1:7447"] },
  transport: {
    shared_memory: { enabled: true }
  }
}
```

---

#### QUIC transport (encrypted, low-latency)

Zenoh 1.9.0 supports QUIC with stream multiplexing for better priority isolation.

**TOML:**
```toml
mode = "client"
connect = { endpoints = ["quic/192.168.1.100:7447"] }
```

---

### Starting a local router for testing

If you don't have a Zenoh infrastructure, you can run a local router using the official Zenoh daemon:

```shell
# Install via cargo
cargo install zenohd

# Start with default settings (listens on 0.0.0.0:7447)
zenohd
```

Then connect Zenoh Hammer in client mode pointing to `tcp/127.0.0.1:7447`.

---

## Usage overview

### Subscribe page

1. Enter a **key expression** (e.g. `demo/**` or `sensors/temperature`).
2. Click **subscribe**. Incoming messages appear in the list with timestamps and frequency.
3. Click a message to inspect its payload, encoding, QoS metadata, and raw hex bytes.

### Put page

1. Enter a **key expression** and select an **encoding**.
2. Edit the payload in the editor (text, JSON, or raw bytes).
3. Click **put** to publish.

### Get page

1. Enter a **key expression** and optional **parameters** (selector parameters after `?`).
2. Configure timeout, consolidation, and target.
3. Click **get**. Replies appear in the list below.

---

## Saving and loading app state

The full application state — all configured subscribers, puts, gets, and the list of session config file paths — can be saved to a `.json` file via **File → Save** and restored with **File → Load**. This makes it easy to maintain ready-to-use test setups for different systems.
