from pathlib import Path

path = Path('src/hardware_ingest.rs')
text = path.read_text()
text = text.replace(
    '    artifact_name: String,\n    discovered_at: u64,\n',
    '    artifact_name: String,\n    artifact_size_bytes: u64,\n    discovered_at: u64,\n',
    1,
)
text = text.replace(
    '        artifact_name: candidate.name.clone(),\n        discovered_at,\n',
    '        artifact_name: candidate.name.clone(),\n        artifact_size_bytes: candidate.size_bytes,\n        discovered_at,\n',
    1,
)
text = text.replace(
    'artifact_name={}\\ndiscovered_at={}\\nfinished_at={}\\nstatus={}\\n",\n',
    'artifact_name={}\\nartifact_size_bytes={}\\ndiscovered_at={}\\nfinished_at={}\\nstatus={}\\n",\n',
    1,
)
text = text.replace(
    '        record.artifact_name,\n        record.discovered_at,\n',
    '        record.artifact_name,\n        record.artifact_size_bytes,\n        record.discovered_at,\n',
    1,
)
text = text.replace(
    '        "dispatch_ref", "artifact_id", "run_id", "artifact_name", "discovered_at",\n',
    '        "dispatch_ref", "artifact_id", "run_id", "artifact_name", "artifact_size_bytes", "discovered_at",\n',
    1,
)
text = text.replace(
    '        artifact_name: required(&fields, "artifact_name")?.to_owned(),\n        discovered_at,\n',
    '        artifact_name: required(&fields, "artifact_name")?.to_owned(),\n        artifact_size_bytes: parse_nonzero_u64("artifact_size_bytes", required(&fields, "artifact_size_bytes")?)?,\n        discovered_at,\n',
    1,
)
text = text.replace(
    '        || expected.artifact_name != actual.artifact_name\n',
    '        || expected.artifact_name != actual.artifact_name\n        || expected.artifact_size_bytes != actual.artifact_size_bytes\n',
    1,
)
path.write_text(text)
print('ORCH6c preflight audit binding applied')
