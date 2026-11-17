#!/usr/bin/env python3
"""vi.lproj/Localizable.strings: the Windows translation table, then Mac-only strings on top."""
import re, sys
src, extra, out = sys.argv[1:4]
text = open(src, encoding='utf8').read()
table = text[:text.index('// Titles with numbers')]
pairs = {}
for m in re.finditer(r"(?:'((?:[^'\\]|\\.)*)'|\b([A-Za-z_][\w]*))\s*:\s*'((?:[^'\\]|\\.)*)'", table):
    pairs[m.group(1) if m.group(1) is not None else m.group(2)] = m.group(3)
for line in open(extra, encoding='utf8'):
    if '=' in line.rstrip('\n'):
        k, v = line.rstrip('\n').split('=', 1) if line.count('=') == 1 else line.rstrip('\n').rsplit('=', 1)
        pairs[k] = v
esc = lambda s: s.replace('\\\'', "'").replace('\\', '\\\\').replace('"', '\\"')
with open(out, 'w', encoding='utf8') as f:
    for k, v in pairs.items():
        f.write(f'"{esc(k)}" = "{esc(v)}";\n')
print(len(pairs), 'strings')
