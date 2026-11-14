"""静态读取准确客户端类型继承关系，为实体空间载荷生成版本目录，不执行客户端。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
from zipfile import ZipFile

VERSIONS = {
    '1.12.2': (1343, '8ada07da5ee77dad3527bd7278fbd05ee1fc8a597813b216a871a2d7d64cc64f',
               {'vq': 'leash', 'adp': 'vex', 'adi': 'shulker'}),
    '1.20.1': (3465, '56b71336d2b4fdffd197f56595b0da93e32a946f78f382a299b8f4b92758bb0f',
               {'bfz': 'sleeping', 'bgb': 'leash', 'brm': 'bee', 'bsm': 'turtle',
                'bwq': 'vex', 'bwe': 'phantom', 'bwd': 'patrol', 'byh': 'trader', 'bwi': 'shulker'}),
    '1.21.1': (3955, '499f6897d1837516680f3114072d8106e11c9adcd933fe5cf051b551089b0c99',
               {'btn': 'sleeping', 'btk': 'leash', 'cff': 'bee', 'cgf': 'turtle',
                'ckr': 'vex', 'ckf': 'phantom', 'cke': 'patrol', 'cmq': 'trader', 'ckj': 'shulker'}),
}

# 保存字段由准确版本的实际基类 / 接口绑定，具体 UUID 编码集中在 Rust 适配层。
REFERENCES = {
    '1.12.2': {'wb': ['tame_owner'], 'aao': ['tame_owner'], 'zv': ['love'],
        'adf': ['anger'], 'acl': ['item'], 'adu': ['converter'], 'aer': ['bullet'],
        've': ['owner_entity'], 'aej': ['owner_entity'], 'aeo': ['llama_owner'], 'aev': ['named_owner']},
    '1.20.1': {'bgv': ['tame_owner'], 'btk': ['tame_owner'], 'brl': ['love'],
        'bgg': ['anger'], 'bvh': ['item'], 'bww': ['converter', 'gossip'], 'bzi': ['bullet'],
        'bfg': ['owner_entity'], 'byy': ['owner_entity'], 'bzg': ['owner_entity'],
        'brv': ['trusted'], 'byb': ['gossip'], 'bxs': ['warden']},
    '1.21.1': {'bul': ['tame_owner'], 'chi': ['tame_owner'], 'cfe': ['love'],
        'btt': ['anger'], 'cjh': ['item'], 'ckx': ['converter', 'gossip'], 'cns': ['bullet'],
        'bsn': ['owner_entity'], 'cnh': ['owner_entity'], 'cnp': ['owner_entity'],
        'cfo': ['trusted'], 'cmk': ['gossip'], 'cmb': ['warden']},
}


def parents(raw):
    """读取常量池和 class 头部的父类 / 接口索引，不加载 Java 类型。"""
    if raw[:4] != bytes.fromhex('cafebabe'):
        raise ValueError('不是 Java class')
    count = struct.unpack_from('>H', raw, 8)[0]
    pool = [None] * count
    at, index = 10, 1
    while index < count:
        tag = raw[at]
        at += 1
        if tag == 1:
            size = struct.unpack_from('>H', raw, at)[0]
            at += 2
            pool[index] = raw[at:at+size].replace(b'\xc0\x80', b'\0').decode('utf-8', 'surrogatepass')
            at += size
        elif tag in (3, 4):
            at += 4
        elif tag in (5, 6):
            at += 8
            index += 1
        elif tag in (7, 8, 16, 19, 20):
            pool[index] = struct.unpack_from('>H', raw, at)[0]
            at += 2
        elif tag in (9, 10, 11, 12, 17, 18):
            at += 4
        elif tag == 15:
            at += 3
        else:
            raise ValueError(f'未知常量池标签 {tag}')
        index += 1
    parent, count = struct.unpack_from('>HH', raw, at+4)
    interfaces = struct.unpack_from('>'+'H'*count, raw, at+8)
    return [pool[pool[i]] for i in [parent, *interfaces] if i]


def generate(version, client, assets):
    data_version, expected, bases = VERSIONS[version]
    if hashlib.sha256(client.read_bytes()).hexdigest() != expected:
        raise ValueError('客户端与准确版本不符')
    codes = {}

    def read(name):
        code = subprocess.check_output(['javap', '-classpath', str(client), '-p', '-c', name], text=True)
        codes[name] = hashlib.sha256(code.encode()).hexdigest()
        return code

    if data_version == 1343:
        code = read('vi')
        origins = dict(re.findall(
            r'// String ([a-z0-9_]+)\s+\d+: ldc(?:_w)?\s+#\d+\s+// class ([\w$]+)'
            r'\s+(?:\d+: ldc(?:_w)?\s+#\d+\s+// String [^\n]+'
            r'|\d+: getstatic\s+#\d+\s+// Field afe\$a\.\w+:Lafe\$a;'
            r'\s+\d+: invokevirtual\s+#\d+\s+// Method afe\$a\.b:\(\)Ljava/lang/String;)'
            r'\s+\d+: invokestatic\s+#\d+'
            r'\s+// Method a:\(ILjava/lang/String;Ljava/lang/Class;Ljava/lang/String;\)V', code))
        if not origins:
            raise ValueError('旧实体注册为空')
        if len(origins) != len(re.findall(r'// Method a:\(ILjava/lang/String;Ljava/lang/Class;Ljava/lang/String;\)V', code)):
            raise ValueError('存在未解释或重复的旧实体注册')
        origins = {'minecraft:'+name: cls for name, cls in origins.items()}
    else:
        registry = 'bfn' if data_version == 3465 else 'bsx'
        code = read(registry)
        fields = dict((field, cls) for cls, field in re.findall(
            r'public static final '+registry+r'<([\w$]+)> ([\w$]+);', code))
        proof = json.loads((assets / f'entity-dimensions-{data_version}.proof.json').read_text(encoding='utf-8'))
        if proof['client_sha256'] != expected:
            raise ValueError('实体来源目录与客户端不符')
        origins = {name: fields[value['field']] for name, value in proof['origins'].items()}
        for name, value in proof['origins'].items():
            if value['class'] is not None and value['class'] != origins[name]:
                raise ValueError(f'实体实际注册类型不一致：{name}')

    ancestors, hashes = {}, {}
    with ZipFile(client) as archive:
        def chain(name):
            if name in ancestors:
                return ancestors[name]
            result = {name}
            if '/' not in name:
                raw = archive.read(name+'.class')
                hashes[name] = hashlib.sha256(raw).hexdigest()
                for parent in parents(raw):
                    result.update(chain(parent))
            ancestors[name] = result
            return result

        entities = {name: sorted({rule for base, rule in bases.items() if base in chain(cls)})
                    for name, cls in sorted(origins.items())}
        references = {name: sorted({rule for base, rules in REFERENCES[version].items()
                                   if base in chain(cls) for rule in rules})
                      for name, cls in sorted(origins.items())}
    output = assets / f'entity-spatial-{data_version}.json'
    output.write_text(json.dumps({'data_version': data_version, 'entities': entities},
                                ensure_ascii=False, separators=(',', ':'))+'\n', encoding='utf-8')
    output.with_suffix('.proof.json').write_text(json.dumps({
        'source': '准确客户端实际类型注册、class 头部继承关系与已核实的保存格式；没有执行客户端',
        'client_sha256': expected, 'bases': bases, 'origins': origins,
        'javap_sha256': codes, 'class_sha256': hashes,
        'output_sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
    }, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    output = assets / f'entity-references-{data_version}.json'
    output.write_text(json.dumps({'data_version': data_version, 'entities': references},
                                ensure_ascii=False, separators=(',', ':'))+'\n', encoding='utf-8')
    output.with_suffix('.proof.json').write_text(json.dumps({
        'source': '准确客户端实际类型注册、class 继承关系与已核实关联字段；未执行客户端',
        'client_sha256': expected, 'bases': REFERENCES[version], 'origins': origins,
        'javap_sha256': codes, 'class_sha256': hashes,
        'output_sha256': hashlib.sha256(output.read_bytes()).hexdigest(),
    }, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(f'{version}: {len(entities)} native spatial / reference registrations')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', choices=VERSIONS, required=True)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--assets', type=Path, required=True)
    args = parser.parse_args()
    generate(args.version, args.client, args.assets)
