"""静态提取准确客户端的物品耐火属性；只解析 class 常量池与 javap，不执行客户端。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
from zipfile import ZipFile

VERSIONS = {
    "1.20.1": (3465, "0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838", "cgc", "cfu", "cfu$a", "cpo", "cpn"),
    "1.21.1": (3955, "30c73b1c5da787909b2f73340419fdf13b9def88", "cut", "cul", "cul$a", "dga", "dfy"),
}


def members(raw):
    """只读取实际成员引用；不将类中的同名文本误作调用。"""
    if raw[:4] != bytes.fromhex("cafebabe"):
        raise ValueError("不是 Java class")
    count = struct.unpack_from(">H", raw, 8)[0]
    pool = [None] * count
    at, index = 10, 1
    while index < count:
        tag = raw[at]
        at += 1
        if tag == 1:
            size = struct.unpack_from(">H", raw, at)[0]
            at += 2
            pool[index] = (tag, raw[at:at+size].replace(b"\xc0\x80", b"\0").decode("utf-8", "surrogatepass"))
            at += size
        elif tag in (3, 4):
            at += 4
        elif tag in (5, 6):
            at += 8
            index += 1
        elif tag in (7, 8, 16, 19, 20):
            pool[index] = (tag, struct.unpack_from(">H", raw, at)[0])
            at += 2
        elif tag in (9, 10, 11, 12, 17, 18):
            pool[index] = (tag, *struct.unpack_from(">HH", raw, at))
            at += 4
        elif tag == 15:
            at += 3
        else:
            raise ValueError(f"未解释的常量池标签 {tag}")
        index += 1
    for entry in pool:
        if entry and entry[0] in (9, 10, 11):
            owner = pool[pool[entry[1]][1]][1]
            kind = pool[entry[2]]
            yield entry[0], owner, pool[kind[1]][1], pool[kind[2]][1]


def generate(version, client, catalog_path, output):
    data_version, expected, items, item, properties, blocks, block = VERSIONS[version]
    client_bytes = client.read_bytes()
    if hashlib.sha1(client_bytes).hexdigest() != expected:
        raise ValueError("客户端与准确版本不符")
    catalog_bytes = catalog_path.read_bytes()
    catalog = json.loads(catalog_bytes)
    if catalog["version"] != version or catalog["client_sha1"] != expected:
        raise ValueError("物品目录不是同一准确客户端")
    fire_method = (10, properties, "a", f"()L{properties};")
    callers = []
    component_users = []
    property_users = []
    with ZipFile(client) as archive:
        for entry in archive.infolist():
            if not entry.filename.endswith(".class"):
                continue
            refs = set(members(archive.read(entry)))
            if fire_method in refs:
                callers.append(entry.filename[:-6])
            if (9, "kq", "w", "Lkp;") in refs:
                component_users.append(entry.filename[:-6])
            if (9, properties, "f", "Z") in refs:
                property_users.append(entry.filename[:-6])
    if callers != [items]:
        raise ValueError(f"耐火构造调用者范围变化：{callers}")

    def code(name):
        return subprocess.check_output(["javap", "-p", "-c", "-classpath", str(client), name], text=True)

    item_code, block_code = code(items), code(blocks)
    item_init = item_code.split("  static {};", 1)[1]
    block_init = block_code.split("  static {};", 1)[1]
    fire_call = f"Method {properties}.a:()L{properties};"
    if item_code.count(fire_call) != item_init.count(fire_call):
        raise ValueError("耐火属性出现在未解释的注册辅助方法中")
    if version == "1.20.1" and set(property_users) != {item, properties}:
        raise ValueError(f"耐火属性字段访问范围变化：{property_users}")
    block_segments = {}
    previous = 0
    for match in re.finditer(r"putstatic\s+#\d+\s+// Field ([\w$]+):L" + block + ";", block_init):
        block_segments[match[1]] = block_init[previous:match.end()]
        previous = match.end()
    resistant = set()
    observed = []
    previous = 0
    for match in re.finditer(r"putstatic\s+#\d+\s+// Field ([\w$]+):L" + item + ";", item_init):
        segment = item_init[previous:match.end()]
        previous = match.end()
        if fire_call not in segment:
            continue
        names = re.findall(r"// String ([a-z0-9_:/.-]+)\s", segment)
        if not names:
            fields = re.findall(r"// Field " + blocks + r"\.([\w$]+):L" + block + ";", segment)
            if len(fields) != 1:
                raise ValueError(f"耐火方块物品没有唯一实际方块源：{match[1]}")
            names = re.findall(r"// String ([a-z0-9_:/.-]+)\s", block_segments[fields[0]])
        if len(names) != 1:
            raise ValueError(f"耐火注册没有唯一实际 ID：{match[1]} {names}")
        name = names[0] if ":" in names[0] else "minecraft:" + names[0]
        if name not in catalog["items"] or name in resistant:
            raise ValueError(f"耐火注册与完整物品目录不一致：{name}")
        resistant.add(name)
        observed.append({"field": match[1], "id": name})
    if not resistant:
        raise ValueError("未提取到实际耐火注册")
    if len(resistant) != item_init.count(fire_call):
        raise ValueError("耐火构造调用与实际注册并非一一对应")
    result = {"data_version": data_version, "items": {name: name in resistant for name in sorted(catalog["items"])}}
    output.write_text(json.dumps(result, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    proof = {
        "source": "准确客户端 class 成员引用与静态注册 javap；没有执行客户端或编译参照类",
        "client_sha256": hashlib.sha256(client_bytes).hexdigest(),
        "catalog_sha256": hashlib.sha256(catalog_bytes).hexdigest(),
        "items_javap_sha256": hashlib.sha256(item_code.encode()).hexdigest(),
        "blocks_javap_sha256": hashlib.sha256(block_code.encode()).hexdigest(),
        "fire_constructor_callers": callers,
        "component_key_users": component_users,
        "property_field_users": property_users,
        "fire_registrations": observed,
        "output_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
    }
    output.with_suffix(".proof.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{version}: {len(result['items'])} 个实际物品，{len(resistant)} 个耐火属性；组件引用 {component_users}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", choices=VERSIONS, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--catalog", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    generate(args.version, args.client, args.catalog, args.output)


if __name__ == "__main__":
    main()
