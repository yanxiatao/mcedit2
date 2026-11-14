"""从准确的官方 1.20.1 客户端与映射静态提取完整方块实体注册关系。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

CLIENT_SHA1 = "0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838"
MAPPINGS_SHA1 = "6c48521eed01fe2e8ecdadbd5ae348415f3c47da"


def generate(client, mappings, javap, output):
    client = client.resolve(strict=True)
    raw = client.read_bytes()
    mapping_bytes = mappings.read_bytes()
    if hashlib.sha1(raw).hexdigest() != CLIENT_SHA1:
        raise ValueError("客户端必须为准确的官方 1.20.1")
    if hashlib.sha1(mapping_bytes).hexdigest() != MAPPINGS_SHA1:
        raise ValueError("映射必须为同一官方 1.20.1 的 client_mappings")
    mapping_text = mapping_bytes.decode("utf-8")

    def mapped_class(name, field_type):
        match = re.search(r"^" + re.escape(name) + r" -> ([\w$]+):\n((?:[ \t].*\n)*)", mapping_text, re.M)
        if not match:
            raise ValueError("映射中缺少类：" + name)
        fields = dict((obfuscated, original.lower()) for original, obfuscated in re.findall(
            r"^    " + re.escape(field_type) + r" ([A-Z_]+) -> ([\w$]+)$", match[2], re.M))
        return match[1], fields

    blocks_class, block_fields = mapped_class("net.minecraft.world.level.block.Blocks", "net.minecraft.world.level.block.Block")
    type_class, type_fields = mapped_class("net.minecraft.world.level.block.entity.BlockEntityType", "net.minecraft.world.level.block.entity.BlockEntityType")

    def initializer(name):
        code = subprocess.check_output([str(javap), "-p", "-c", "-classpath", str(client), name], text=True, encoding="utf-8", errors="strict")
        return code.split("  static {};", 1)[1]

    blocks = initializer(blocks_class)
    registered_blocks = {}
    previous = 0
    for field in re.finditer(r"putstatic\s+#[0-9]+\s+// Field ([\w$]+):", blocks):
        if field[1] in block_fields:
            name = block_fields[field[1]]
            segment = blocks[previous:field.end()]
            if re.search(r"// String " + re.escape(name) + r"\s", segment):
                registered_blocks[field[1]] = "minecraft:" + name
        previous = field.end()
    types = {}
    seen = set()
    code = initializer(type_class)
    pattern = (r"ldc(?:_w)?\s+#[0-9]+\s+// String ([a-z_]+)\s+(.*?)"
               r"putstatic\s+#[0-9]+\s+// Field ([\w$]+):L" + re.escape(type_class) + r";")
    for name, body, field in re.findall(pattern, code, re.S):
        if field in seen or type_fields[field] != name:
            raise ValueError("实体注册 ID 与字段映射不一致：" + name)
        seen.add(field)
        fields = re.findall(r"getstatic\s+#[0-9]+\s+// Field " + re.escape(blocks_class) + r"\.([\w$]+):", body)
        values = [registered_blocks[field] for field in fields]
        if not values or len(values) != len(set(values)):
            raise ValueError("实体类型没有唯一的方块集合：" + name)
        if len(fields) != len(re.findall(r"\baastore\b", body)):
            raise ValueError("方块数组写入与静态字段数量不同：" + name)
        types["minecraft:" + name] = values
    if seen != set(type_fields) or len(types) != 41:
        raise ValueError(f"类型覆盖不完整：提取 {len(types)} / 映射 {len(type_fields)}")
    result = {
        "data_version": 3465,
        "client_sha1": CLIENT_SHA1,
        "client_sha256": hashlib.sha256(raw).hexdigest(),
        "mappings_sha1": MAPPINGS_SHA1,
        "source": "官方 1.20.1 BlockEntityType 静态注册数组、Blocks 实际注册 ID 与官方客户端映射",
        "types": dict(sorted(types.items())),
    }
    with output.open("w", encoding="utf-8", newline="\n") as target:
        target.write(json.dumps(result, ensure_ascii=False, separators=(",", ":")) + "\n")
    print(f"已核实 {len(types)} 个方块实体类型、{len(set(v for values in types.values() for v in values))} 个方块")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-jar", type=Path, required=True)
    parser.add_argument("--mappings", type=Path, required=True)
    parser.add_argument("--javap", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    generate(args.client_jar, args.mappings, args.javap, args.output)
