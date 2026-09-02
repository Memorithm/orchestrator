from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()
old = "    let declared_steps = attempt.binding.declared_steps();\n"
new = "    let declared_steps = plan.steps.len();\n"
assert text.count(old) == 1, text.count(old)
path.write_text(text.replace(old, new, 1))
