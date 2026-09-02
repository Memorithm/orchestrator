from pathlib import Path

path = Path("src/hardware_evidence.rs")
text = path.read_text()
old = '''    if fields.len() != ALLOWED.len() {
        if let Some(unknown) = fields
            .keys()
            .find(|name| !ALLOWED.contains(&name.as_str()))
        {
            return Err(format!("unknown hardware evidence field: {unknown}"));
        }
    }
'''
new = '''    if fields.len() != ALLOWED.len()
        && let Some(unknown) = fields
            .keys()
            .find(|name| !ALLOWED.contains(&name.as_str()))
    {
        return Err(format!("unknown hardware evidence field: {unknown}"));
    }
'''
count = text.count(old)
if count != 1:
    raise SystemExit(f"expected exactly one collapsible-if anchor, got {count}")
path.write_text(text.replace(old, new, 1))
