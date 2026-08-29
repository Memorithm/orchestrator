#!/usr/bin/env python3
from pathlib import Path

path = Path("src/main.rs")
source = path.read_text()


def replace_once(old: str, new: str, label: str) -> None:
    global source
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    source = source.replace(old, new, 1)

replace_once(
    "mod merge_policy;\nmod publication;\nmod state;\nmod trajectory;\n",
    "mod health;\nmod merge_policy;\nmod publication;\nmod state;\nmod trajectory;\n",
    "health module",
)

replace_once(
    '''fn usage(program: &str) {
''',
    '''fn health() -> ExitCode {
    let report = health::inspect(&default_data_root(), unix_timestamp());
    println!("{}", report.text);
    if report.degraded {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn usage(program: &str) {
''',
    "health command",
)

replace_once(
    '''    eprintln!("  {program} doctor");
    eprintln!("  {program} scan [organization]");
''',
    '''    eprintln!("  {program} doctor");
    eprintln!("  {program} health");
    eprintln!("  {program} status");
    eprintln!("  {program} scan [organization]");
''',
    "health usage",
)

replace_once(
    '''        Some("doctor") => doctor(),
        Some("scan") => scan(&organization_arg(&mut args)),
''',
    '''        Some("doctor") => doctor(),
        Some("health") | Some("status") => health(),
        Some("scan") => scan(&organization_arg(&mut args)),
''',
    "health dispatch",
)

path.write_text(source)
