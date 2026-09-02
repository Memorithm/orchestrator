from pathlib import Path

main = Path('src/main.rs')
s = main.read_text()
needle = 'mod evidence;\nmod hardware_dispatch;\n'
assert s.count(needle) == 1, s.count(needle)
s = s.replace(needle, 'mod evidence;\nmod hardware_capability;\nmod hardware_dispatch;\n', 1)
main.write_text(s)

path = Path('src/hardware_dispatch.rs')
s = path.read_text()
needle = 'use crate::hardware_evidence::HardwareEvidenceRequest;\n'
assert s.count(needle) == 1, s.count(needle)
s = s.replace(
    needle,
    'use crate::hardware_capability::{self, HardwareCapabilityOutcome};\nuse crate::hardware_evidence::HardwareEvidenceRequest;\n',
    1,
)

needle = '    let dispatching_contents = serialize_record(&record);\n    claim_dispatch(&state_path, &dispatching_contents)?;\n'
assert s.count(needle) == 1, s.count(needle)
replacement = '''    match hardware_capability::check_schedulable_with_program(request, &config, now, program)? {\n        HardwareCapabilityOutcome::Schedulable => {}\n        HardwareCapabilityOutcome::Deferred(reason) => {\n            return Ok(HardwareDispatchOutcome::Deferred(reason));\n        }\n    }\n\n    let dispatching_contents = serialize_record(&record);\n    claim_dispatch(&state_path, &dispatching_contents)?;\n'''
s = s.replace(needle, replacement, 1)

needle = '''        fs::write(\n            directory.join("jetson-thor-real-device.state"),\n            "v1\\nmode=github_workflow\\nrepository=Memorithm/hardware-ci\\nworkflow=dispatch.yml\\nref=main\\n",\n        )\n        .unwrap();\n    }\n\n    #[test]\n    fn missing_config_defers_without_remote_mutation() {\n'''
assert s.count(needle) == 1, s.count(needle)
replacement = '''        fs::write(\n            directory.join("jetson-thor-real-device.state"),\n            "v1\\nmode=github_workflow\\nrepository=Memorithm/hardware-ci\\nworkflow=dispatch.yml\\nref=main\\n",\n        )\n        .unwrap();\n        let capability_directory = root.join("config/hardware-capabilities");\n        fs::create_dir_all(&capability_directory).unwrap();\n        fs::write(\n            capability_directory.join("jetson-thor-real-device.state"),\n            "v1\\nmode=hosted\\nrepository=Memorithm/hardware-ci\\n",\n        )\n        .unwrap();\n    }\n\n    #[test]\n    fn missing_config_defers_without_remote_mutation() {\n'''
s = s.replace(needle, replacement, 1)
path.write_text(s)

print('ORCH6d integration applied')
