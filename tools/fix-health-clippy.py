#!/usr/bin/env python3
from pathlib import Path

path = Path("src/health.rs")
source = path.read_text()
replacements = [
    (
        '''    let mut counts = WorkCounts::default();
    counts.total = files.len();
''',
        '''    let mut counts = WorkCounts {
        total: files.len(),
        ..WorkCounts::default()
    };
''',
        "work counts initializer",
    ),
    (
        '''    let mut counts = PublicationCounts::default();
    counts.total = files.len();
''',
        '''    let mut counts = PublicationCounts {
        total: files.len(),
        ..PublicationCounts::default()
    };
''',
        "publication counts initializer",
    ),
]
for old, new, label in replacements:
    count = source.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    source = source.replace(old, new, 1)
path.write_text(source)
