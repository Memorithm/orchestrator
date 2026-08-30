from pathlib import Path

path = Path("src/main.rs")
text = path.read_text()
old = '''    if !check_ollama() {
        return Err("Ollama is not reachable".to_owned());
    }
    if !is_local_ollama_model(&config.model) || !check_model_available(&config.model) {'''
new = '''    if !check_ollama() {
        return Err("Ollama is not reachable".to_owned());
    }
    if !check_bwrap_sandbox() {
        return Err("bubblewrap process sandbox is installed but unusable".to_owned());
    }
    if !is_local_ollama_model(&config.model) || !check_model_available(&config.model) {'''
if text.count(old) != 1:
    raise SystemExit("runtime_preflight anchor changed")
path.write_text(text.replace(old, new, 1))
