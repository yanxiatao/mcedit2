"""静态读取准确 1.12.2 客户端，交叉核对原 MCEdit2 的全部方块实体关系。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


CLIENT_SHA256 = "8ada07da5ee77dad3527bd7278fbd05ee1fc8a597813b216a871a2d7d64cc64f"


def generate(client, reference, output):
    client = client.resolve(strict=True)
    if hashlib.sha256(client.read_bytes()).hexdigest() != CLIENT_SHA256:
        raise ValueError("客户端必须是准确的官方 1.12.2，不能推断其他版本的混淆名")
    cache = {}

    def bytecode(name):
        if name not in cache:
            cache[name] = subprocess.check_output(
                ["javap", "-classpath", str(client), "-p", "-c", name],
                text=True, encoding="utf-8", errors="strict",
            )
        return cache[name]

    def original(name):
        return json.loads(subprocess.check_output([
            "git", "show", f"{reference}:src/mceditlib/blocktypes/{name}_1_12.json"
        ]))

    tile_types = dict((class_name, "minecraft:" + name) for name, class_name in re.findall(
        r"// String ([a-z_]+)\s+\d+: ldc(?:_w)?\s+#\d+\s+// class ([\w$]+)"
        r"\s+\d+: invokestatic\s+#\d+\s+// Method a:\(Ljava/lang/String;Ljava/lang/Class;\)V",
        bytecode("avj"),
    ))
    if len(tile_types) != 25:
        raise ValueError("官方 1.12.2 方块实体注册表数量不匹配")
    states = {entry["blockState"]: entry for entry in original("minecraft_raw")}
    expected = {}
    for block_id, _, state in original("idmapping_raw"):
        entry = states[state]
        if entry.get("hasEntity"):
            expected.setdefault(entry["internalName"], set()).add(block_id)
    if len(expected) != 48:
        raise ValueError("原 BlockDumper 的方块实体方块覆盖数量不匹配")
    registry = bytecode("aow")
    types = {}
    for block, ids in sorted(expected.items()):
        name = block.split(":", 1)[1]
        match = re.search(
            r"(?:bipush|sipush)\s+(\d+)\s+\d+: ldc(?:_w)?\s+#\d+\s+// String "
            + re.escape(name) + r"\s+\d+: new\s+#\d+\s+// class ([\w$]+)", registry,
        )
        if not match or ids != {int(match[1])}:
            raise ValueError(f"官方数字注册 ID 与原 BlockDumper 不符：{block}")
        class_name = match[2]
        visited = set()
        while True:
            if class_name in visited:
                raise ValueError(f"方块工厂继承成环：{block}")
            visited.add(class_name)
            code = bytecode(class_name)
            constructed = set(re.findall(r"new\s+#\d+\s+// class ([\w$]+)", code)) & tile_types.keys()
            if constructed:
                if len(constructed) != 1:
                    raise ValueError(f"方块工厂有多个实体类型，需逐项核对：{block}")
                entity = tile_types[constructed.pop()]
                types.setdefault(entity, []).extend([block, f"numeric:{match[1]}"])
                break
            parent = re.search(r"public (?:abstract )?class \S+ extends (\S+)", code)
            if not parent:
                raise ValueError(f"方块工厂没有已注册实体构造：{block}")
            class_name = parent[1]
    if set(types) != set(tile_types.values()):
        raise ValueError("方块工厂没有覆盖全部已注册方块实体类型")
    output.write_text(json.dumps({
        "data_version": 1343,
        "client_sha256": CLIENT_SHA256,
        "source": "官方 1.12.2 avj/aow 静态注册与方块工厂；MCEdit2 " + reference + " BlockDumper 交叉核对",
        "types": {key: sorted(values) for key, values in sorted(types.items())},
    }, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"已核实 {len(types)} 个方块实体类型、{len(expected)} 个方块 ID")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-jar", type=Path, required=True)
    parser.add_argument("--reference", required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    generate(arguments.client_jar, arguments.reference, arguments.output)
