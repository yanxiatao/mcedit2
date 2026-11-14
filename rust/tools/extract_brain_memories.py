"""静态提取准确客户端记忆注册的保存 codec；不运行客户端或参照类。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

VERSIONS = {
    '1.20.1': (3465, 'bpb', 'hd.a', 'gu.a', 'hy.a',
               '56b71336d2b4fdffd197f56595b0da93e32a946f78f382a299b8f4b92758bb0f'),
    '1.21.1': (3955, 'ccs', 'jl.b', 'jd.a', 'kg.a',
               '499f6897d1837516680f3114072d8106e11c9adcd933fe5cf051b551089b0c99'),
}


def generate(version, client, assets):
    number, owner, global_pos, block_pos, uuid, expected = VERSIONS[version]
    if hashlib.sha256(client.read_bytes()).hexdigest() != expected:
        raise ValueError('客户端与准确版本不符')
    code = subprocess.check_output(['javap', '-classpath', str(client), '-p', '-c', owner], text=True)
    init = code.split('  static {};', 1)[1]
    entries, evidence = {}, {}
    for segment in re.split(r'\s*\d+: putstatic\s+[^\n]+\n', init):
        match = re.search(r'// String ([a-z_]+)', segment)
        if not match:
            continue
        name = 'minecraft:'+match[1]
        if name in entries:
            raise ValueError('记忆注册重复：'+name)
        refs = re.findall(r'// (?:Field|(?:Interface)?Method) (.*)', segment)
        if 'Ljava/lang/String;Lcom/mojang/serialization/Codec;' not in segment:
            kind = 'transient'
        elif any(r.startswith(global_pos+':') for r in refs):
            kind = 'global_list' if any('Codec.list' in r or 'Codec.listOf' in r for r in refs) else 'global'
        elif any(r.startswith(block_pos+':') for r in refs):
            kind = 'position'
        elif any(r.startswith(uuid+':') for r in refs):
            kind = 'uuid'
        elif any('Codec.INT:' in r for r in refs):
            kind = 'int'
        elif any('Codec.LONG:' in r for r in refs):
            kind = 'long'
        elif any('Codec.BOOL:' in r for r in refs):
            kind = 'bool'
        elif any('Codec.unit:' in r for r in refs) or (number == 3955 and any(r.startswith('azs.b:') for r in refs)):
            kind = 'unit'
        else:
            raise ValueError(f'未解释的记忆 codec：{name} {refs}')
        entries[name] = kind
        evidence[name] = refs
    if not entries:
        raise ValueError('实际记忆注册为空')
    output = assets / f'brain-memories-{number}.json'
    output.write_text(json.dumps({'data_version': number, 'memories': entries}, ensure_ascii=False, separators=(',', ':'))+'\n', encoding='utf-8')
    output.with_suffix('.proof.json').write_text(json.dumps({
        'source': '准确客户端 MemoryModuleType 静态注册与实际 codec 引用；未执行客户端',
        'client_sha256': expected, 'owner': owner,
        'javap_sha256': hashlib.sha256(code.encode()).hexdigest(), 'registrations': evidence,
        'output_sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
    }, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(f'{version}: {len(entries)} memory registrations')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', choices=VERSIONS, required=True)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--assets', type=Path, required=True)
    args = parser.parse_args()
    generate(args.version, args.client, args.assets)
