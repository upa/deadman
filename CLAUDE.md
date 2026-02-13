# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`deadman` is a curses-based network monitoring tool that tracks host status using ICMP ping. It was originally designed for temporary networks at events like Interop Tokyo ShowNet.

## Running deadman

```bash
# Basic usage
./deadman deadman.conf

# Asynchronous mode (faster, concurrent pinging)
./deadman -a deadman.conf
./deadman --async-mode deadman.conf

# Custom RTT scale (default: 10ms)
./deadman -s 20 deadman.conf
```

**Keyboard controls while running:**
- `q` - Quit gracefully
- `r` - Refresh all targets
- `Ctrl+C` - Quit (sends SIGINT, handled gracefully)

**Reloading configuration:**
```bash
# Send SIGHUP to reload config without losing history
kill -HUP <pid>
```

## Configuration File Format

Each line defines a monitoring target:

```
NAME    ADDRESS    [options]
```

**Separator:**
```
---
```

**Direct ping:**
```
googleDNS    8.8.8.8
kame6        2001:2f0:0:8800::1:1
```

**SSH relay ping:**
```
google-via-ssh    173.194.117.176    relay=X.X.X.X os=Linux user=USER key=KEYPATH
```

**SNMP ping (RFC4560):**
```
googleDNS-via-snmp    8.8.8.8    relay=X.X.X.X via=snmp community=public
```

**Network namespace (Linux, requires root):**
```
googleDNS-via-netns    8.8.8.8    relay=netns1 via=netns
```

**VRF (Linux, requires root):**
```
googleDNS-via-vrf    8.8.8.8    relay=vrf1 via=vrf
```

**TCP-based ping (Linux, requires root for raw sockets):**
```
wide-tcp80    203.178.136.59    tcp=dstport:80
```

## Code Architecture

### Single-file Structure

All code is in the `deadman` Python3 script (~970 lines):

- **PingResult** (lines 82-91): Data class for ping results (success, errcode, rtt, ttl)
- **PingTarget** (lines 93-280): Represents a target host and encapsulates ping logic
  - Supports multiple relay methods: direct, SSH, SNMP, netns, VRF
  - Handles OS-specific ping commands (Linux, Darwin/macOS, FreeBSD, RouterOS)
  - Parses ping output to extract RTT and TTL
- **CursesCtrl** (lines 537-620): Terminal UI controller using ncurses
  - Renders title, host info, target list
  - Displays RTT bar graphs
  - Handles color coding (green=up, red=down)
  - Manages window resize (SIGWINCH)
- **Deadman** (lines 693-823): Main application controller
  - Loads and parses config file
  - Manages target list and ping execution
  - Implements synchronous and asynchronous ping loops
  - Handles SIGHUP for config reload
  - Spawns key handler thread for 'r' refresh

### Signal Handling

- **SIGINT** (Ctrl+C): Graceful shutdown - sets quit flag
- **SIGTERM**: Graceful shutdown - sets quit flag
- **SIGWINCH**: Window resize - triggers terminal redraw
- **SIGHUP**: Config reload - preserves target history

The quit flag (`QUIT_FLAG`) is checked in both sync and async main loops, allowing the application to exit cleanly when either:
1. User presses 'q' key
2. SIGINT or SIGTERM signal is received

### Ping Execution Modes

**Synchronous mode** (`main()` at line 737):
- Pings targets sequentially with `PING_INTERVAL` (0.05s) between each
- Full cycle interval: `PING_ALLTARGET_INTERVAL` (1s)

**Asynchronous mode** (`async_main()` at line 775):
- Uses `concurrent.futures.ThreadPoolExecutor`
- Pings all targets concurrently
- Updates UI as results arrive

### OS-Specific Ping Commands

The script detects OS via `uname -s` and uses appropriate ping syntax:
- **Linux**: `ping -c 1 -W <timeout>`
- **Darwin/macOS**: `ping -c 1 -W <timeout_ms>`
- **FreeBSD**: `ping -c 1 -W <timeout_ms>`
- **RouterOS**: Uses REST API instead of ping command

Ping parsing handles OS-specific output formats for RTT and TTL extraction.

## Development Notes

### Adding New Relay Methods

1. Add relay detection in `PingTarget.__init__()` (line 95)
2. Implement relay-specific ping command in `PingTarget.do_ping()` (line 119)
3. Add OS-specific command variants in `pingcmd()` function
4. Update config file documentation

### Modifying UI Layout

Constants at top of file control layout:
- `MAX_HOSTNAME_LENGTH = 20`
- `MAX_ADDRESS_LENGTH = 40`
- `RESULT_STR_LENGTH = 10`
- `RTT_SCALE = 10` (milliseconds per bar unit)

Color pairs defined in `main()` (line 959+):
- `DEFAULT_COLOR`: Normal text
- `UP_COLOR`: Green for reachable hosts
- `DOWN_COLOR`: Red for unreachable hosts
- `FAST_COLOR`: Bright green for RTT < 10ms
- `MEDIUM_COLOR`: Yellow for RTT 10-50ms
- `SLOW_COLOR`: Orange for RTT 50-100ms
- `VSLOW_COLOR`: Red for RTT > 100ms
- `HEADER_COLOR`: Cyan for headers
- `SEPARATOR_COLOR`: Blue for separators

### Visual Indicators

**Status Symbols:**
- `✓` - Host is up (0% packet loss)
- `⏱` - Partial packet loss
- `✗` - Host is down (100% packet loss)
- `⚠` - SSH connection failed

**RTT Bar Characters:**
- `▁` - Very fast (< RTT_SCALE × 1)
- `▂` - Fast (< RTT_SCALE × 2)
- `▃` - Good (< RTT_SCALE × 3)
- `▄` - Medium (< RTT_SCALE × 4)
- `▅` - Slower (< RTT_SCALE × 5)
- `▆` - Slow (< RTT_SCALE × 6)
- `▇` - Very slow (< RTT_SCALE × 7)
- `█` - Extremely slow (≥ RTT_SCALE × 7)

Each bar character is colored based on RTT speed (green → yellow → orange → red)

**UI Elements:**
- Box-drawing characters for borders and separators (╔═╗ ─ │)
- Unicode arrow for current target indicator: `▶`
- Color-coded RTT values in the stats column

### Testing Different Ping Methods

Requires specific environments:
- SSH relay: Requires SSH access to relay host
- SNMP: Requires SNMP-enabled device supporting RFC4560
- netns/VRF: Linux-only, requires root privileges
- TCP ping: Requires root for raw socket access

### Error Codes

```python
PING_SUCCESS      = 0
PING_FAILED       = -1
PING_SSH_TIMEOUT  = -2
PING_SSH_FAILED   = -3
```

## Dependencies

- Python 3 (uses `#!/usr/bin/env python3`)
- Standard library only (no external packages)
- OS utilities: `ping`, `ssh` (for relay), `snmpget` (for SNMP mode)

## Performance Optimizations

### Fast Startup

The script has been optimized for instant startup:

1. **Lazy DNS Resolution**: Hostname lookups are deferred until actually needed (0.5s timeout)
2. **Fast OS Detection**: Uses `platform.system()` instead of subprocess `uname -s`
3. **IP Version Caching**: Results of `whichipversion()` are cached to avoid repeated DNS queries
4. **Immediate UI Display**: Title and UI render before target loading
5. **Loading Indicator**: Shows "Loading targets..." while parsing config and resolving IPs

**Performance Impact:**
- Startup time reduced from ~1-3 seconds to <100ms
- UI appears immediately even with slow DNS
- Config with many targets loads without blocking UI

### Caching Mechanisms

```python
_hostname_cache      # Cached hostname (lazy-loaded)
_hostinfo_cache      # Cached host info with IP (lazy-loaded, 0.5s timeout)
_ipversion_cache     # IP version lookup results (addr -> 4/6/False)
```

IP version detection tries `inet_pton()` first (instant for raw IPs) before falling back to `getaddrinfo()` (DNS lookup).

## Logging

When `LOGDIR` is set, deadman logs ping results to files. By default, logging is disabled (`LOGDIR = ""`).
