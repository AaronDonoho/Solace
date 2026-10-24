//! What this computer needs to run Solace: the app's first-run checklist asks here.

use super::agents;
use super::on_path;
use crate::migrate;
use serde_json::{Value, json};
use std::fs;
use std::process::Command;

/// What a check reads from this computer, so tests can give their own answers.
pub(super) struct Probe<'a> {
    pub on_path: &'a dyn Fn(&str) -> bool,
    pub output: &'a dyn Fn(&str, &[&str]) -> Option<String>,
    pub agent: &'a dyn Fn() -> agents::AgentCheck,
    pub sandbox: &'a dyn Fn() -> bool,
    pub platform: Platform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Macos,
    Debian,
    Arch,
    OtherLinux,
    Other,
}

#[derive(Clone, Copy)]
enum Tool {
    Gleam,
    Erlang,
    Agent,
    Bubblewrap,
    Git,
}

const TOOLS: [Tool; 5] = [Tool::Gleam, Tool::Erlang, Tool::Agent, Tool::Bubblewrap, Tool::Git];

const AGENT_RELEASE: &str = "Or download Codex from https://github.com/openai/codex/releases.";

/// Every check, run now: nothing is cached, so **Check again** sees a newly installed tool.
pub(super) fn check() -> Value {
    let platform = platform();
    json!({
        "platform": platform_name(platform),
        "items": items(&Probe {
            on_path: &on_path,
            output: &command_output,
            agent: &agents::check_now,
            sandbox: &migrate::sandbox_works,
            platform,
        }),
    })
}

fn items(p: &Probe) -> Vec<Value> {
    let mut out = vec![item(p, Tool::Gleam), item(p, Tool::Erlang), item(p, Tool::Agent), item(p, Tool::Git)];
    if platform_linux(p.platform) {
        out.insert(3, item(p, Tool::Bubblewrap));
    }
    if out.iter().any(|item| item["ok"] == false) {
        if let Some(item) = homebrew(p) {
            out.insert(0, item);
        }
    }
    out
}

fn problem(p: &Probe, tool: Tool) -> Option<String> {
    match tool {
        Tool::Gleam => {
            if !(p.on_path)("gleam") {
                return Some("Gleam formats, checks, tests and builds your projects.".into());
            }
            match (p.output)("gleam", &["--version"]).and_then(|text| version(&text)) {
                Some(v) if v >= (1, 18, 1) => None,
                Some((major, minor, patch)) => Some(format!("Gleam {major}.{minor}.{patch} is too old; Solace needs 1.18.1 or newer.")),
                None => Some("Solace couldn't read Gleam's version; it needs 1.18.1 or newer.".into()),
            }
        }
        Tool::Erlang => {
            if !(p.on_path)("erl") {
                return Some("Gleam projects run on Erlang.".into());
            }
            let eval = "io:format(\"~s\",[erlang:system_info(otp_release)]),halt().";
            match (p.output)("erl", &["-noshell", "-eval", eval]).and_then(|text| otp(&text)) {
                Some(n) if n >= 29 => None,
                Some(n) => Some(format!("Erlang/OTP {n} is too old; Solace needs OTP 29 or newer.")),
                None => Some("Solace couldn't read Erlang's version; it needs OTP 29 or newer.".into()),
            }
        }
        Tool::Agent => match (p.agent)() {
            agents::AgentCheck::Ready => None,
            agents::AgentCheck::Missing => Some("An agent does the work you assign on the board; install one and sign in.".into()),
            agents::AgentCheck::SignedOut { label, .. } => Some(format!("{label} is installed but not signed in, so no task can run.")),
        },
        Tool::Bubblewrap => {
            if !(p.on_path)("bwrap") {
                Some(
                    concat!(
                        "bubblewrap is not installed. On Linux it makes the folder a migration protects read-only ",
                        "to Solace's commands, and runs Rust commands without network access, writing only target/ and Cargo.lock."
                    )
                    .into(),
                )
            } else if !(p.sandbox)() {
                Some("bubblewrap is installed but cannot run here, so Solace's commands run without that protection.".into())
            } else {
                None
            }
        }
        Tool::Git => (!(p.on_path)("git")).then(|| "Git keeps your project's history and lets you switch branches.".into()),
    }
}

fn item(p: &Probe, tool: Tool) -> Value {
    let problem = problem(p, tool);
    let steps = match problem.is_some() {
        true => steps(p, tool),
        false => Vec::new(),
    };
    item_with(tool_id(tool), tool_label(tool), problem, steps)
}

fn tool_id(tool: Tool) -> &'static str {
    match tool {
        Tool::Gleam => "gleam",
        Tool::Erlang => "erlang",
        Tool::Agent => "agent",
        Tool::Bubblewrap => "bubblewrap",
        Tool::Git => "git",
    }
}

fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Gleam => "Gleam",
        Tool::Erlang => "Erlang/OTP",
        Tool::Agent => "An agent",
        Tool::Bubblewrap => "bubblewrap",
        Tool::Git => "Git",
    }
}

fn platform_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Macos => "macos",
        Platform::Debian => "debian",
        Platform::Arch => "arch",
        Platform::OtherLinux => "linux",
        Platform::Other => "other",
    }
}

fn platform_linux(platform: Platform) -> bool {
    matches!(platform, Platform::Debian | Platform::Arch | Platform::OtherLinux)
}

fn install(platform: Platform, tool: Tool) -> Vec<Step> {
    match (platform, tool) {
        (Platform::Macos, Tool::Gleam) => vec![command("brew install gleam")],
        (Platform::Macos, Tool::Erlang) => vec![command("brew install erlang")],
        (Platform::Macos, Tool::Agent) => vec![
            command("brew install codex"),
            note("Or install Claude Code:"),
            command("curl -fsSL https://claude.ai/install.sh | bash"),
            note("Then run it once and sign in."),
        ],
        (Platform::Macos, Tool::Git) => vec![command("brew install git")],
        (Platform::Macos, Tool::Bubblewrap) => vec![note("bubblewrap is not needed on macOS.")],
        (Platform::Debian, Tool::Gleam) => vec![
            note(concat!(
                "Download the x86_64-unknown-linux-musl or aarch64-unknown-linux-musl archive from ",
                "https://github.com/gleam-lang/gleam/releases, then run:"
            )),
            command("tar xf gleam-*-unknown-linux-musl.tar.gz"),
            command("sudo install -m 755 gleam /usr/local/bin/gleam"),
        ],
        (Platform::Debian, Tool::Erlang) => vec![
            note("Ubuntu's and Debian's erlang package can be older than OTP 29. Build it from source:"),
            command("sudo apt install -y build-essential curl libssl-dev libncurses-dev m4"),
            command("cd /tmp"),
            command("curl -LO https://github.com/erlang/otp/releases/download/OTP-29.0/otp_src_29.0.tar.gz"),
            command("tar xf otp_src_29.0.tar.gz && cd otp_src_29.0"),
            command("./configure --prefix=/usr/local && make && sudo make install"),
            command("cd -"),
        ],
        (Platform::Debian, Tool::Agent) => {
            vec![command("curl -fsSL https://claude.ai/install.sh | bash"), note(AGENT_RELEASE), note("Then run it once and sign in.")]
        }
        (Platform::Debian, Tool::Bubblewrap) => vec![command("sudo apt install -y bubblewrap")],
        (Platform::Debian, Tool::Git) => vec![command("sudo apt install -y git")],
        (Platform::Arch, Tool::Gleam) => vec![command("sudo pacman -S --needed gleam")],
        (Platform::Arch, Tool::Erlang) => vec![command("sudo pacman -S --needed erlang")],
        (Platform::Arch, Tool::Agent) => {
            vec![command("curl -fsSL https://claude.ai/install.sh | bash"), note(AGENT_RELEASE), note("Then run it once and sign in.")]
        }
        (Platform::Arch, Tool::Bubblewrap) => vec![command("sudo pacman -S --needed bubblewrap")],
        (Platform::Arch, Tool::Git) => vec![command("sudo pacman -S --needed git")],
        (Platform::OtherLinux | Platform::Other, Tool::Gleam) => vec![note("Install it from https://gleam.run/install/")],
        (Platform::OtherLinux | Platform::Other, Tool::Erlang) => vec![note("Install it from https://www.erlang.org/downloads")],
        (Platform::OtherLinux | Platform::Other, Tool::Agent) => vec![
            note("Install Codex from https://github.com/openai/codex/releases or Claude Code from https://claude.ai/install.sh."),
            note("Then run it once and sign in."),
        ],
        (Platform::OtherLinux | Platform::Other, Tool::Bubblewrap) => {
            vec![note("Install it from https://github.com/containers/bubblewrap")]
        }
        (Platform::OtherLinux | Platform::Other, Tool::Git) => vec![note("Install it from https://git-scm.com/downloads")],
    }
}

fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "linux") {
        fs::read_to_string("/etc/os-release").map(|text| from_os_release(&text)).unwrap_or(Platform::OtherLinux)
    } else {
        Platform::Other
    }
}

fn from_os_release(text: &str) -> Platform {
    let names = format!("{} {}", field(text, "ID").unwrap_or(""), field(text, "ID_LIKE").unwrap_or(""));
    if names.split_whitespace().any(|name| name == "arch") {
        Platform::Arch
    } else if names.split_whitespace().any(|name| name == "debian" || name == "ubuntu") {
        Platform::Debian
    } else {
        Platform::OtherLinux
    }
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"'))
    })
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

fn version(text: &str) -> Option<(u32, u32, u32)> {
    text.split_whitespace().find_map(|part| {
        let mut pieces = part.trim_start_matches('v').split('.');
        let major: u32 = pieces.next()?.parse().ok()?;
        let minor: u32 = pieces.next()?.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()?;
        let patch: u32 = pieces.next().and_then(|piece| piece.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()).unwrap_or(0);
        Some((major, minor, patch))
    })
}

fn otp(text: &str) -> Option<u32> {
    text.split_whitespace().find_map(|part| part.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yes(_: &str) -> bool {
        true
    }

    fn no(_: &str) -> bool {
        false
    }

    fn nothing(_: &str, _: &[&str]) -> Option<String> {
        None
    }

    fn current(_: &str, _: &[&str]) -> Option<String> {
        Some("gleam 1.18.1 OTP 29".into())
    }

    fn agent_yes() -> agents::AgentCheck {
        agents::AgentCheck::Ready
    }

    fn agent_no() -> agents::AgentCheck {
        agents::AgentCheck::Missing
    }

    fn probe(platform: Platform) -> Probe<'static> {
        Probe { on_path: &yes, output: &current, agent: &agent_yes, sandbox: &sandbox_yes, platform }
    }

    #[test]
    fn every_check_passes_when_the_tools_are_there() {
        let mac = items(&probe(Platform::Macos));
        assert_eq!(mac.len(), 4);
        assert!(mac.iter().all(|item| item["ok"] == true));
        assert!(!mac.iter().any(|item| item["id"] == "bubblewrap"));

        let linux = items(&probe(Platform::Debian));
        assert_eq!(linux.len(), 5);
        assert!(linux.iter().all(|item| item["ok"] == true));
    }

    #[test]
    fn each_missing_tool_says_why_and_how_to_install_it() {
        let p = Probe { on_path: &no, output: &nothing, agent: &agent_no, sandbox: &sandbox_no, platform: Platform::Arch };
        let items = items(&p);
        assert_eq!(items.len(), 5);
        for item in &items {
            assert_eq!(item["ok"], false, "{item}");
            assert!(!item["why"].as_str().unwrap_or("").is_empty(), "{item}");
            assert!(!item["steps"].as_array().unwrap_or(&Vec::new()).is_empty(), "{item}");
        }
    }

    #[test]
    fn an_old_gleam_names_both_versions() {
        let old = |_: &str, _: &[&str]| Some("gleam 1.17.0".to_string());
        let p = Probe { on_path: &yes, output: &old, agent: &agent_yes, sandbox: &sandbox_yes, platform: Platform::Macos };
        let items = items(&p);
        let gleam = items.iter().find(|item| item["id"] == "gleam").unwrap();
        let why = gleam["why"].as_str().unwrap();
        assert!(why.contains("1.17.0"));
        assert!(why.contains("1.18.1"));
    }

    #[test]
    fn an_old_erlang_names_both_versions() {
        let old = |_: &str, _: &[&str]| Some("27".to_string());
        let p = Probe { on_path: &yes, output: &old, agent: &agent_yes, sandbox: &sandbox_yes, platform: Platform::Macos };
        let items = items(&p);
        let erlang = items.iter().find(|item| item["id"] == "erlang").unwrap();
        let why = erlang["why"].as_str().unwrap();
        assert!(why.contains("27"));
        assert!(why.contains("29"));
    }

    #[test]
    fn an_unreadable_version_is_reported() {
        let p = Probe { on_path: &yes, output: &nothing, agent: &agent_yes, sandbox: &sandbox_yes, platform: Platform::Macos };
        let items = items(&p);
        let gleam = items.iter().find(|item| item["id"] == "gleam").unwrap();
        assert!(gleam["why"].as_str().unwrap().contains("couldn't read"));
        let erlang = items.iter().find(|item| item["id"] == "erlang").unwrap();
        assert!(erlang["why"].as_str().unwrap().contains("couldn't read"));
    }

    #[test]
    fn versions_are_read_from_the_tools() {
        assert_eq!(version("gleam 1.18.1"), Some((1, 18, 1)));
        assert_eq!(version("gleam 1.18"), Some((1, 18, 0)));
        assert_eq!(version("gleam"), None);
        assert_eq!(otp("29"), Some(29));
        assert_eq!(otp("27\n"), Some(27));
        assert_eq!(otp("not a number"), None);
    }

    #[test]
    fn the_system_is_read_from_os_release() {
        assert_eq!(from_os_release("ID=ubuntu\nID_LIKE=debian\n"), Platform::Debian);
        assert_eq!(from_os_release("ID=debian\n"), Platform::Debian);
        assert_eq!(from_os_release("ID=arch\n"), Platform::Arch);
        assert_eq!(from_os_release("ID=manjaro\nID_LIKE=\"arch\"\n"), Platform::Arch);
        assert_eq!(from_os_release("ID=fedora\n"), Platform::OtherLinux);
    }

    #[test]
    fn each_system_gets_its_own_install_step() {
        assert_eq!(install(Platform::Macos, Tool::Gleam)[0].text, "brew install gleam");
        assert!(install(Platform::Debian, Tool::Erlang).iter().any(|step| step.command && step.text.contains("otp_src_29.0.tar.gz")));
        assert_eq!(install(Platform::Arch, Tool::Bubblewrap)[0].text, "sudo pacman -S --needed bubblewrap");
        assert_eq!(install(Platform::OtherLinux, Tool::Git)[0].text, "Install it from https://git-scm.com/downloads");
        assert!(install(Platform::Macos, Tool::Agent).iter().any(|step| step.text.contains("claude.ai")));
        assert!(install(Platform::Debian, Tool::Agent).iter().any(|step| step.text.contains("codex/releases")));
        for platform in [Platform::Macos, Platform::Debian, Platform::Arch, Platform::OtherLinux, Platform::Other] {
            for tool in TOOLS {
                assert!(!install(platform, tool).is_empty());
            }
        }
    }

    #[test]
    fn the_checklist_names_the_runtime_tools() {
        let labels: Vec<&str> = TOOLS.iter().map(|tool| tool_label(*tool)).collect();
        assert_eq!(labels, vec!["Gleam", "Erlang/OTP", "An agent", "bubblewrap", "Git"]);
    }

    fn sandbox_yes() -> bool {
        true
    }

    fn sandbox_no() -> bool {
        false
    }

    fn agent_signed_out() -> agents::AgentCheck {
        agents::AgentCheck::SignedOut { name: "codex", label: "Codex" }
    }

    #[test]
    fn a_signed_out_agent_says_which_one_and_how_to_sign_in() {
        let p = Probe { on_path: &yes, output: &current, agent: &agent_signed_out, sandbox: &sandbox_yes, platform: Platform::Debian };
        let items = items(&p);
        let agent = items.iter().find(|item| item["id"] == "agent").unwrap();
        assert_eq!(agent["ok"], false);
        assert!(agent["why"].as_str().unwrap().contains("Codex is installed but not signed in"));
        let steps = agent["steps"].as_array().unwrap();
        assert_eq!(steps[0]["text"], "codex");
        assert_eq!(steps[0]["command"], true);
        assert_eq!(steps[1]["command"], false);
    }

    #[test]
    fn an_installed_but_blocked_bubblewrap_is_not_called_missing() {
        let only_bwrap = |program: &str| program == "bwrap";
        let blocked = || false;
        let p = Probe { on_path: &only_bwrap, output: &current, agent: &agent_yes, sandbox: &blocked, platform: Platform::Debian };
        let items = items(&p);
        let bwrap = items.iter().find(|item| item["id"] == "bubblewrap").unwrap();
        let why = bwrap["why"].as_str().unwrap();
        assert!(why.contains("installed but cannot run"));
        assert!(!why.contains("not installed"));
        assert_eq!(bwrap["steps"][0]["text"], "bwrap --dev-bind / / true");
    }

    #[test]
    fn commands_and_their_explanations_are_separate_steps() {
        for platform in [Platform::Macos, Platform::Debian, Platform::Arch, Platform::OtherLinux, Platform::Other] {
            for tool in TOOLS {
                for step in install(platform, tool) {
                    if step.command {
                        assert!(!step.text.contains(", or "), "{}", step.text);
                        assert!(!step.text.contains("Install it from"), "{}", step.text);
                    }
                }
            }
        }
        let debian = install(Platform::Debian, Tool::Gleam);
        assert!(debian.iter().any(|step| !step.command && step.text.contains("gleam-lang/gleam/releases")));
        assert!(debian.iter().any(|step| step.command && step.text == "tar xf gleam-*-unknown-linux-musl.tar.gz"));
        assert!(debian.iter().any(|step| step.command && step.text == "sudo install -m 755 gleam /usr/local/bin/gleam"));
        let erlang = install(Platform::Debian, Tool::Erlang);
        assert!(erlang.iter().any(|step| step.command && step.text.contains("otp_src_29.0.tar.gz")));
        assert!(erlang.iter().any(|step| step.command && step.text == "./configure --prefix=/usr/local && make && sudo make install"));
        assert!(!erlang.iter().any(|step| step.text == "sudo apt install -y erlang"));
    }

    #[test]
    fn missing_bubblewrap_names_what_it_protects() {
        let p = Probe { on_path: &no, output: &current, agent: &agent_yes, sandbox: &sandbox_no, platform: Platform::Debian };
        let items = items(&p);
        let bwrap = items.iter().find(|item| item["id"] == "bubblewrap").unwrap();
        let why = bwrap["why"].as_str().unwrap();
        assert!(why.contains("target/"));
        assert!(why.contains("Cargo.lock"));
        assert!(!why.contains("outside their build folder"));
    }

    #[test]
    fn macos_without_homebrew_says_how_to_install_it() {
        let p = Probe { on_path: &no, output: &nothing, agent: &agent_no, sandbox: &sandbox_no, platform: Platform::Macos };
        let items = items(&p);
        assert_eq!(items.len(), 5);
        let brew = &items[0];
        assert_eq!(brew["id"], "homebrew");
        assert_eq!(brew["label"], "Homebrew");
        assert_eq!(brew["ok"], false);
        assert!(brew["why"].as_str().unwrap().contains("installs the tools below"));
        assert_eq!(
            brew["steps"][0]["text"],
            "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
        );
        assert_eq!(brew["steps"][0]["command"], true);
        assert_eq!(brew["steps"][1]["command"], false);
    }

    #[test]
    fn homebrew_is_not_asked_for_when_everything_else_passes() {
        let brew_missing = |program: &str| program != "brew";
        let p = Probe { on_path: &brew_missing, output: &current, agent: &agent_yes, sandbox: &sandbox_yes, platform: Platform::Macos };
        let mac = items(&p);
        assert_eq!(mac.len(), 4);
        assert!(!mac.iter().any(|item| item["id"] == "homebrew"));
        assert!(!items(&probe(Platform::Debian)).iter().any(|item| item["id"] == "homebrew"));
    }


}

#[derive(Clone, Copy)]
struct Step {
    text: &'static str,
    command: bool,
}

const fn command(text: &'static str) -> Step {
    Step { text, command: true }
}

const fn note(text: &'static str) -> Step {
    Step { text, command: false }
}

fn steps_json(steps: Vec<Step>) -> Vec<Value> {
    steps.into_iter().map(|step| json!({"text": step.text, "command": step.command})).collect()
}

fn steps(p: &Probe, tool: Tool) -> Vec<Step> {
    match tool {
        Tool::Agent => match (p.agent)() {
            agents::AgentCheck::Missing => install(p.platform, tool),
            agents::AgentCheck::SignedOut { name, .. } => {
                vec![command(name), note("Sign in when it starts, then press Check again.")]
            }
            agents::AgentCheck::Ready => Vec::new(),
        },
        Tool::Bubblewrap if (p.on_path)("bwrap") => vec![
            command("bwrap --dev-bind / / true"),
            note("If that fails, this system blocks bubblewrap; Solace's commands run without it."),
        ],
        _ => install(p.platform, tool),
    }
}

fn item_with(id: &str, label: &str, problem: Option<String>, steps: Vec<Step>) -> Value {
    let missing = problem.is_some();
    json!({
        "id": id,
        "label": label,
        "ok": !missing,
        "why": problem.unwrap_or_default(),
        "steps": steps_json(steps),
    })
}

fn homebrew(p: &Probe) -> Option<Value> {
    (p.platform == Platform::Macos && !(p.on_path)("brew")).then(|| {
        item_with(
            "homebrew",
            "Homebrew",
            Some("Homebrew installs the tools below on macOS.".into()),
            vec![
                command("/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""),
                note("Follow the installer's PATH steps, then restart Solace."),
            ],
        )
    })
}
