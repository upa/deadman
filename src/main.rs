// deadman-rs : 設定ファイル駆動の ICMP/Ping 監視 TUI
//
// 元ネタ: upa/deadman (Python) を Rust (ratatui + tokio) へ移植したもの。
// 設定ファイル(TOML)を読み、グループ単位で重要度ラベルを付けて一覧表示し、
// 各ターゲットの `via` フィールド(icmp/tcp/ssh/snmp/netns/vrf)で
// 到達確認の方法を切り替えながら、ライブで結果テーブルを描画する。
//
// build : cargo build --release
// run   : deadman-rs [-s SCALE_MS] <config.toml>
//   -s SCALE_MS : スパークライン1段あたりの RTT(ms)。既定 10。

use ping_async::{IcmpEchoRequestor, IcmpEchoStatus};
use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::mpsc;

// ---------------- 定数 ----------------
const PING_INTERVAL: Duration = Duration::from_secs(1);
const PING_TIMEOUT: Duration = Duration::from_secs(1);
const SSH_CONNECT_TIMEOUT: u64 = 3;
const SSH_TIMEOUT: Duration = Duration::from_secs(6);
const SNMP_TIMEOUT: Duration = Duration::from_secs(4);
const CMD_TIMEOUT: Duration = Duration::from_secs(3);
const HISTORY_MAX: usize = 200;

// ---------------- 設定ファイル ----------------
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    group: Vec<GroupCfg>,
}

#[derive(Deserialize)]
struct GroupCfg {
    label: String,
    #[serde(default)]
    target: Vec<TargetCfg>,
}

#[derive(Deserialize)]
struct TargetCfg {
    name: String,
    address: String,
    via: Option<String>,
    relay: Option<String>,
    os: Option<String>,
    user: Option<String>,
    key: Option<String>,
    community: Option<String>,
    tcp_port: Option<u16>,
}

// ---------------- ping 方式 ----------------
#[derive(Clone, Copy)]
enum OsKind {
    Linux,
    Darwin,
}

impl OsKind {
    fn parse(s: Option<&str>) -> OsKind {
        match s.map(|x| x.to_ascii_lowercase()).as_deref() {
            Some("darwin") | Some("macos") | Some("mac") => OsKind::Darwin,
            _ => OsKind::Linux,
        }
    }
}

#[derive(Clone)]
enum Method {
    Icmp,
    Tcp {
        port: u16,
    },
    Ssh {
        relay: String,
        user: Option<String>,
        key: Option<String>,
        os: OsKind,
    },
    Snmp {
        relay: String,
        community: String,
    },
    Netns {
        relay: String,
    },
    Vrf {
        relay: String,
    },
}

fn method_from_cfg(t: &TargetCfg) -> Result<Method, String> {
    let via = t.via.as_deref().map(str::to_ascii_lowercase);
    match via.as_deref() {
        None | Some("icmp") | Some("ping") => Ok(Method::Icmp),
        Some("tcp") => {
            let port = t
                .tcp_port
                .ok_or_else(|| format!("target '{}': via=tcp requires tcp_port", t.name))?;
            Ok(Method::Tcp { port })
        }
        Some("ssh") => Ok(Method::Ssh {
            relay: require(&t.relay, &t.name, "ssh", "relay")?,
            user: t.user.clone(),
            key: t.key.clone(),
            os: OsKind::parse(t.os.as_deref()),
        }),
        Some("snmp") => Ok(Method::Snmp {
            relay: require(&t.relay, &t.name, "snmp", "relay")?,
            community: require(&t.community, &t.name, "snmp", "community")?,
        }),
        Some("netns") => Ok(Method::Netns {
            relay: require(&t.relay, &t.name, "netns", "relay")?,
        }),
        Some("vrf") => Ok(Method::Vrf {
            relay: require(&t.relay, &t.name, "vrf", "relay")?,
        }),
        Some(other) => Err(format!("target '{}': unknown via '{}'", t.name, other)),
    }
}

fn require(field: &Option<String>, name: &str, via: &str, key: &str) -> Result<String, String> {
    field
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("target '{}': via={} requires '{}'", name, via, key))
}

// ---------------- 1回分の結果 ----------------
#[derive(Clone, Copy)]
enum Outcome {
    Up(f64),      // RTT(ms)
    Down,         // 応答なし → 'X'
    RelayTimeout, // ssh 接続タイムアウト → 't'
    RelayError,   // 中継/コマンド失敗 → 's'
}

struct PingMsg {
    idx: usize,
    outcome: Outcome,
}

// ---------------- ホストの実行時状態 ----------------
struct Target {
    name: String,
    addr: IpAddr,
    method: Method,
    up: Option<bool>, // None = 未計測
    snt: u64,
    loss: u64,
    rtt: f64,
    tot: f64,
    history: VecDeque<Outcome>, // 先頭が最新
}

impl Target {
    fn apply(&mut self, o: Outcome) {
        self.snt += 1;
        match o {
            Outcome::Up(ms) => {
                self.up = Some(true);
                self.rtt = ms;
                self.tot += ms;
            }
            _ => {
                self.up = Some(false);
                self.loss += 1;
                self.rtt = 0.0;
            }
        }
        self.history.push_front(o);
        while self.history.len() > HISTORY_MAX {
            self.history.pop_back();
        }
    }

    fn avg(&self) -> f64 {
        let ok = self.snt - self.loss;
        if ok == 0 {
            0.0
        } else {
            self.tot / ok as f64
        }
    }

    fn loss_rate(&self) -> f64 {
        if self.snt == 0 {
            0.0
        } else {
            self.loss as f64 * 100.0 / self.snt as f64
        }
    }

    fn reset(&mut self) {
        self.up = None;
        self.snt = 0;
        self.loss = 0;
        self.rtt = 0.0;
        self.tot = 0.0;
        self.history.clear();
    }
}

// ---------------- 表示用グループ ----------------
struct GroupView {
    label: String,
    level: usize,
    members: Vec<usize>, // targets への添字
}

fn heading_level(label: &str) -> usize {
    label.chars().take_while(|c| *c == '#').count().max(1)
}

fn clean_label(label: &str) -> String {
    label.trim_start_matches('#').trim().to_string()
}

fn label_style(level: usize) -> Style {
    match level {
        1 => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        2 => Style::default().add_modifier(Modifier::BOLD),
        _ => Style::default().add_modifier(Modifier::DIM),
    }
}

// ---------------- スパークライン ----------------
fn spark_line(history: &VecDeque<Outcome>, scale: f64, width: usize) -> Line<'static> {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let spans: Vec<Span> = history
        .iter()
        .take(width.max(1))
        .map(|o| match o {
            Outcome::Up(ms) => {
                let lvl = ((ms / scale).floor() as usize).min(BARS.len() - 1);
                Span::styled(BARS[lvl].to_string(), Style::default().fg(Color::Green))
            }
            Outcome::Down => Span::styled("X".to_string(), Style::default().fg(Color::Red)),
            Outcome::RelayTimeout => {
                Span::styled("t".to_string(), Style::default().fg(Color::Yellow))
            }
            Outcome::RelayError => {
                Span::styled("s".to_string(), Style::default().fg(Color::Magenta))
            }
        })
        .collect();
    Line::from(spans)
}

// ---------------- 出力パース ----------------
fn parse_ping_rtt(out: &str) -> Option<f64> {
    let start = out.find("time=")? + "time=".len();
    let num: String = out[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f64>().ok()
}

fn parse_snmpping_rtt(out: &str) -> Option<f64> {
    // snmpping: "rtt min/avg/max/stddev = 1.2/3.4/5.6/0.1 ms"
    let marker = out.find("min/avg/max")?;
    let eq = out[marker..].find('=')? + marker + 1;
    let num: String = out[eq..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f64>().ok()
}

// ---------------- コマンド組み立て ----------------
fn ping_cmd(os: OsKind, addr: &IpAddr, timeout_secs: u64) -> Vec<String> {
    match os {
        // 近年の iputils ping は v4/v6 兼用。-c 回数, -W 応答待ち秒
        OsKind::Linux => vec![
            "ping".into(),
            "-c".into(),
            "1".into(),
            "-W".into(),
            timeout_secs.to_string(),
            addr.to_string(),
        ],
        OsKind::Darwin => {
            let prog = if addr.is_ipv6() { "ping6" } else { "ping" };
            vec![
                prog.into(),
                "-c".into(),
                "1".into(),
                "-t".into(),
                timeout_secs.to_string(),
                addr.to_string(),
            ]
        }
    }
}

fn build_argv(method: &Method, addr: &IpAddr, timeout_secs: u64) -> Option<Vec<String>> {
    match method {
        Method::Ssh {
            relay,
            user,
            key,
            os,
        } => {
            let mut v = vec![
                "ssh".into(),
                "-o".into(),
                format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT),
                "-o".into(),
                "StrictHostKeyChecking=no".into(),
                "-o".into(),
                "BatchMode=yes".into(),
            ];
            if let Some(k) = key {
                v.push("-i".into());
                v.push(k.clone());
            }
            if let Some(u) = user {
                v.push("-l".into());
                v.push(u.clone());
            }
            v.push(relay.clone());
            v.extend(ping_cmd(*os, addr, timeout_secs));
            Some(v)
        }
        Method::Netns { relay } => {
            let mut v = vec!["ip".into(), "netns".into(), "exec".into(), relay.clone()];
            v.extend(ping_cmd(OsKind::Linux, addr, timeout_secs));
            Some(v)
        }
        Method::Vrf { relay } => {
            let mut v = vec!["ip".into(), "vrf".into(), "exec".into(), relay.clone()];
            v.extend(ping_cmd(OsKind::Linux, addr, timeout_secs));
            Some(v)
        }
        Method::Snmp { relay, community } => Some(vec![
            "snmpping".into(),
            "-Cc1".into(),
            "-v".into(),
            "2c".into(),
            "-c".into(),
            community.clone(),
            relay.clone(),
            addr.to_string(),
        ]),
        Method::Icmp | Method::Tcp { .. } => None, // ネイティブ処理
    }
}

// ---------------- サブプロセス実行 ----------------
async fn run_subprocess(
    argv: &[String],
    total_timeout: Duration,
    is_ssh: bool,
    is_snmp: bool,
) -> Outcome {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .env("LC_ALL", "C")
        .kill_on_drop(true);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Outcome::RelayError, // コマンドが無い等
    };

    let output = match tokio::time::timeout(total_timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(_)) => return Outcome::RelayError,
        Err(_) => {
            return if is_ssh {
                Outcome::RelayTimeout
            } else {
                Outcome::Down
            }
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let rtt = if is_snmp {
        parse_snmpping_rtt(&text)
    } else {
        parse_ping_rtt(&text)
    };
    match rtt {
        Some(ms) => Outcome::Up(ms),
        None => {
            if is_ssh && !text.contains("PING") && !text.contains("bytes from") {
                Outcome::RelayError
            } else {
                Outcome::Down
            }
        }
    }
}

// ---------------- 1回分の probe（ICMP 以外）----------------
async fn probe_once(method: &Method, addr: IpAddr) -> Outcome {
    match method {
        Method::Tcp { port } => {
            let start = Instant::now();
            match tokio::time::timeout(PING_TIMEOUT, TcpStream::connect((addr, *port))).await {
                Ok(Ok(_)) => Outcome::Up(start.elapsed().as_secs_f64() * 1000.0),
                Ok(Err(_)) | Err(_) => Outcome::Down,
            }
        }
        Method::Snmp { .. } => {
            let argv = build_argv(method, &addr, PING_TIMEOUT.as_secs().max(1)).unwrap();
            run_subprocess(&argv, SNMP_TIMEOUT, false, true).await
        }
        Method::Ssh { .. } => {
            let argv = build_argv(method, &addr, PING_TIMEOUT.as_secs().max(1)).unwrap();
            run_subprocess(&argv, SSH_TIMEOUT, true, false).await
        }
        Method::Netns { .. } | Method::Vrf { .. } => {
            let argv = build_argv(method, &addr, PING_TIMEOUT.as_secs().max(1)).unwrap();
            run_subprocess(&argv, CMD_TIMEOUT, false, false).await
        }
        Method::Icmp => unreachable!("icmp is handled in worker()"),
    }
}

// ---------------- ホストごとの ping ループ ----------------
async fn worker(idx: usize, method: Method, addr: IpAddr, tx: mpsc::UnboundedSender<PingMsg>) {
    match &method {
        Method::Icmp => {
            // ICMP はネイティブ。raw socket を1本だけ確保して使い回す
            let pinger = match IcmpEchoRequestor::new(addr, None, Some(128), Some(PING_TIMEOUT)) {
                Ok(p) => p,
                Err(_) => {
                    // raw socket を作れない(権限不足など)。失敗を出し続ける
                    loop {
                        if tx
                            .send(PingMsg {
                                idx,
                                outcome: Outcome::Down,
                            })
                            .is_err()
                        {
                            return;
                        }
                        tokio::time::sleep(PING_INTERVAL).await;
                    }
                }
            };
            loop {
                let outcome = match pinger.send().await {
                    Ok(reply) if matches!(reply.status(), IcmpEchoStatus::Success) => {
                        Outcome::Up(reply.round_trip_time().as_secs_f64() * 1000.0)
                    }
                    _ => Outcome::Down,
                };
                if tx.send(PingMsg { idx, outcome }).is_err() {
                    return;
                }
                tokio::time::sleep(PING_INTERVAL).await;
            }
        }
        _ => loop {
            let outcome = probe_once(&method, addr).await;
            if tx.send(PingMsg { idx, outcome }).is_err() {
                return;
            }
            tokio::time::sleep(PING_INTERVAL).await;
        },
    }
}

// ---------------- 設定読み込み ----------------
fn load(path: &str) -> Result<(Vec<Target>, Vec<GroupView>), String> {
    let contents = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
    let cfg: Config =
        toml::from_str(&contents).map_err(|e| format!("failed to parse '{}': {}", path, e))?;

    let mut targets = Vec::new();
    let mut groups = Vec::new();

    for g in &cfg.group {
        let level = heading_level(&g.label);
        let label = clean_label(&g.label);
        let mut members = Vec::new();
        for t in &g.target {
            let addr: IpAddr = t
                .address
                .parse()
                .map_err(|_| format!("target '{}': invalid IP address '{}'", t.name, t.address))?;
            let method = method_from_cfg(t)?;
            members.push(targets.len());
            targets.push(Target {
                name: t.name.clone(),
                addr,
                method,
                up: None,
                snt: 0,
                loss: 0,
                rtt: 0.0,
                tot: 0.0,
                history: VecDeque::new(),
            });
        }
        groups.push(GroupView {
            label,
            level,
            members,
        });
    }
    Ok((targets, groups))
}

// ---------------- 引数 ----------------
struct Args {
    config: String,
    scale: f64,
}

fn parse_args() -> Result<Args, String> {
    const USAGE: &str = "usage: deadman-rs [-s SCALE_MS] <config.toml>";
    let mut config: Option<String> = None;
    let mut scale = 10.0_f64;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-s" | "--scale" => {
                let v = it.next().ok_or("-s requires a value (ms)")?;
                scale = v.parse().map_err(|_| format!("invalid scale '{}'", v))?;
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other if config.is_none() => config = Some(other.to_string()),
            other => return Err(format!("unexpected argument '{}'", other)),
        }
    }
    let config = config.ok_or(USAGE)?;
    if scale <= 0.0 {
        return Err("scale must be > 0".into());
    }
    Ok(Args { config, scale })
}

fn local_host_info() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|h| format!("From: {}", h))
        .unwrap_or_else(|| "From: (unknown)".to_string())
}

// ---------------- 1行ぶんの Row ----------------
fn target_row(t: &Target, scale: f64, history_width: usize) -> Row<'static> {
    let name_style = match t.up {
        Some(false) => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        _ => Style::default(),
    };
    let loss_style = if t.loss_rate() > 0.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let rtt_cell = if matches!(t.up, Some(true)) {
        format!("{:.1}", t.rtt)
    } else {
        "-".to_string()
    };
    let avg_cell = if t.snt > t.loss {
        format!("{:.1}", t.avg())
    } else {
        "-".to_string()
    };

    Row::new(vec![
        Cell::from(t.name.clone()).style(name_style),
        Cell::from(t.addr.to_string()),
        Cell::from(format!("{:.0}%", t.loss_rate())).style(loss_style),
        Cell::from(rtt_cell),
        Cell::from(avg_cell),
        Cell::from(t.snt.to_string()),
        Cell::from(spark_line(&t.history, scale, history_width)),
    ])
}

// ---------------- UI ループ ----------------
fn run_ui(
    terminal: &mut ratatui::DefaultTerminal,
    targets: &mut [Target],
    groups: &[GroupView],
    scale: f64,
    host_info: &str,
    rx: &mut mpsc::UnboundedReceiver<PingMsg>,
) -> std::io::Result<()> {
    let mut spinner = 0usize;
    loop {
        // 1) 溜まった結果を反映
        while let Ok(msg) = rx.try_recv() {
            if let Some(t) = targets.get_mut(msg.idx) {
                t.apply(msg.outcome);
            }
        }
        spinner = spinner.wrapping_add(1);

        // 2) 描画
        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

            let wheel = ['|', '/', '-', '\\'][(spinner / 2) % 4];
            let title = format!(
                " Dead Man   {host}   scale={scale:.0}ms   [q]uit  [r]eset   {wheel}",
                host = host_info,
                scale = scale,
                wheel = wheel
            );
            frame.render_widget(
                Paragraph::new(title).style(Style::default().add_modifier(Modifier::BOLD)),
                chunks[0],
            );

            let fixed: usize = 22 + 26 + 6 + 7 + 7 + 7 + 6;
            let history_width = (chunks[1].width as usize).saturating_sub(fixed).max(10);

            let header = Row::new(vec![
                "HOSTNAME", "ADDRESS", "LOSS", "RTT", "AVG", "SNT", "HISTORY",
            ])
            .style(Style::default().add_modifier(Modifier::BOLD));

            let mut rows: Vec<Row> = Vec::new();
            for (gi, g) in groups.iter().enumerate() {
                if gi > 0 {
                    rows.push(Row::new(vec![Cell::from("")])); // 区切りの空行
                }
                rows.push(Row::new(vec![
                    Cell::from(g.label.clone()).style(label_style(g.level))
                ]));
                for &mi in &g.members {
                    rows.push(target_row(&targets[mi], scale, history_width));
                }
            }

            let widths = [
                Constraint::Length(22),
                Constraint::Length(26),
                Constraint::Length(6),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Min(10),
            ];
            let table = Table::new(rows, widths).header(header).column_spacing(1);
            frame.render_widget(table, chunks[1]);
        })?;

        // 3) 入力（タイムアウト付き）
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                let ctrl_c =
                    k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c');
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    _ if ctrl_c => break,
                    KeyCode::Char('r') => {
                        for t in targets.iter_mut() {
                            t.reset();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(2);
        }
    };

    let (mut targets, groups) = match load(&args.config) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    };
    if targets.is_empty() {
        eprintln!("error: no targets in config");
        std::process::exit(1);
    }

    // ワーカー起動
    let (tx, mut rx) = mpsc::unbounded_channel::<PingMsg>();
    for (idx, t) in targets.iter().enumerate() {
        let tx = tx.clone();
        let method = t.method.clone();
        let addr = t.addr;
        tokio::spawn(worker(idx, method, addr, tx));
    }
    drop(tx);

    let host_info = local_host_info();

    // #[tokio::main] は既定でマルチスレッド。UI ループでこのスレッドが
    // ブロックしても、ping タスクは別ワーカースレッドで動き続ける。
    let mut terminal = ratatui::init();
    let res = run_ui(
        &mut terminal,
        &mut targets,
        &groups,
        args.scale,
        &host_info,
        &mut rx,
    );
    ratatui::restore();

    if let Err(e) = res {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}
