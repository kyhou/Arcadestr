//! Native game fixture. Compile directly with rustc; no shell at launch time.
//! argv: report-path, release-path, exit-code, then arbitrary test arguments.
use std::{env, fs, thread, time::{Duration, Instant}};

fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            ch if ch < ' ' => result.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 { return Err("expected report, release and exit-code arguments".into()); }
    #[cfg(target_os = "linux")]
    let parent_pid: u32 = fs::read_to_string("/proc/self/status")?.lines()
        .find_map(|line| line.strip_prefix("PPid:")).ok_or("missing PPid")?.trim().parse()?;
    #[cfg(target_os = "linux")]
    let no_new_privs: u32 = fs::read_to_string("/proc/self/status")?.lines()
        .find_map(|line| line.strip_prefix("NoNewPrivs:")).ok_or("missing NoNewPrivs")?.trim().parse()?;
    #[cfg(target_os = "linux")]
    let stdio = (0..=2).map(|fd| fs::read_link(format!("/proc/self/fd/{fd}"))
        .map(|path| json_string(&path.to_string_lossy()))).collect::<Result<Vec<_>, _>>()?.join(",");
    #[cfg(not(target_os = "linux"))]
    let parent_pid = 0;
    #[cfg(not(target_os = "linux"))]
    let no_new_privs = 0;
    #[cfg(not(target_os = "linux"))]
    let stdio = String::new();
    let argv = args.iter().map(|value| json_string(value)).collect::<Vec<_>>().join(",");
    let environment = env::vars_os().map(|(key, value)| format!("{}:{}", json_string(&key.to_string_lossy()), json_string(&value.to_string_lossy()))).collect::<Vec<_>>().join(",");
    let report = format!("{{\"pid\":{},\"parent_pid\":{},\"no_new_privs\":{},\"stdio\":[{}],\"argv\":[{}],\"cwd\":{},\"environment\":{{{}}}}}", std::process::id(), parent_pid, no_new_privs, stdio, argv, json_string(&env::current_dir()?.to_string_lossy()), environment);
    let temporary = format!("{}.writing", args[1]);
    fs::write(&temporary, report)?;
    fs::rename(temporary, &args[1])?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while !std::path::Path::new(&args[2]).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    std::process::exit(args[3].parse()?);
}
