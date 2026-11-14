"""静态提取准确客户端的猫变种注册与缺省键；只读取映射和 javap，不执行客户端。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
from zipfile import ZipFile

VERSIONS = {
    "1.20.1": (3465, "56b71336d2b4fdffd197f56595b0da93e32a946f78f382a299b8f4b92758bb0f"),
    "1.21.1": (3955, "499f6897d1837516680f3114072d8106e11c9adcd933fe5cf051b551089b0c99"),
}


def mapped(source, name):
    match = re.search(r"^" + re.escape(name) + r" -> ([\w$]+):\n(.*?)(?=^[\w.$]+ -> |\Z)", source, re.M | re.S)
    if match is None:
        raise ValueError("官方映射缺少类型：" + name)
    return match[1], match[2]


def method(source, header):
    lines = source.splitlines()
    start = next(i for i, line in enumerate(lines) if line.strip() == header)
    end = next((i for i in range(start + 1, len(lines)) if re.match(r"  (?:public|private|protected|static) ", lines[i])), len(lines))
    return "\n".join(lines[start:end])


def generate(version, client, mappings, output):
    data_version, expected = VERSIONS[version]
    raw = client.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError("客户端不是声明的准确版本")
    mapping_bytes = mappings.read_bytes()
    mapping = mapping_bytes.decode("utf-8")
    variant, _ = mapped(mapping, "net.minecraft.world.entity.animal.CatVariant")
    cat, cat_mapping = mapped(mapping, "net.minecraft.world.entity.animal.Cat")
    dumps = {name: subprocess.check_output(["javap", "-classpath", str(client), "-p", "-c", name], text=True) for name in [variant, cat]}
    keys = {}
    key_types = set()
    static = method(dumps[variant], "static {};")
    for match in re.finditer(r"// String ([\w:/.-]+)\n\s*\d+: invokestatic[^\n]*\n\s*\d+: putstatic[^\n]*// Field (\w+):L([\w$]+);", static):
        key = match[1] if ":" in match[1] else "minecraft:" + match[1]
        keys[match[2]] = key
        key_types.add(match[3])
    if not keys or len(key_types) != 1:
        raise ValueError("猫注册键的原生构造格式未识别")
    key_type = key_types.pop()
    variants = {}
    for match in re.finditer(r"getstatic[^\n]*// Field (\w+):L" + re.escape(key_type) + r";\n\s*\d+: ldc(?:_w)?[^\n]*// String (textures/[^\r\n]+\.png)\n\s*\d+: invokestatic[^\n]*// Method \w+:\([^\n]+\)L" + re.escape(variant) + ";", dumps[variant]):
        key = keys[match[1]]
        if key in variants:
            raise ValueError("重复的猫变种注册：" + key)
        texture = match[2].removeprefix("textures/").removesuffix(".png")
        variants[key] = "minecraft:" + texture
    if set(variants) != set(keys.values()):
        raise ValueError("注册键与实际变种绑定未完整对应")
    name = re.search(r"defineSynchedData\([^\n]+ -> (\w+)", cat_mapping)[1]
    parameters = ""
    if version == "1.21.1":
        parameters = mapped(mapping, "net.minecraft.network.syncher.SynchedEntityData$Builder")[0]
    defined = method(dumps[cat], "protected void " + name + "(" + parameters + ");")
    refs = re.findall(r"getstatic[^\n]*// Field (?:(\w+)\.)?(\w+):L" + re.escape(key_type) + ";", defined)
    if len(refs) != 1:
        raise ValueError("猫同步数据的缺省变种入口未识别")
    owner, field = refs[0]
    if not owner:
        assigned = re.search(r"getstatic[^\n]*// Field " + re.escape(variant) + r"\.(\w+):L" + re.escape(key_type) + r";\n\s*\d+: putstatic[^\n]*// Field " + re.escape(field) + ":", method(dumps[cat], "static {};"))
        field = assigned[1]
    elif owner != variant:
        raise ValueError("缺省猫变种来自未识别的注册来源")
    default = keys[field]
    result = {"data_version": data_version, "default": default, "variants": dict(sorted(variants.items()))}
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    with ZipFile(client) as archive:
        members = {name + ".class": hashlib.sha256(archive.read(name + ".class")).hexdigest() for name in dumps}
    proof = {"version": version, "client_sha256": expected, "mappings_sha256": hashlib.sha256(mapping_bytes).hexdigest(),
             "classes": members, "disassembly_sha256": {name: hashlib.sha256(value.encode()).hexdigest() for name, value in dumps.items()},
             "extractor_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "output_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
             "scope": "只读取实际注册键、纹理构造和同步数据缺省键；没有编译或执行客户端"}
    output.with_suffix(".proof.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", choices=VERSIONS, required=True)
    for key in ["client", "mappings", "output"]:
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    generate(args.version, args.client, args.mappings, args.output)
