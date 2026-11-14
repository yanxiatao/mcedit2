"""从准确客户端静态注册提取全实体类型的尺寸、原生眼高及耐火标记，不执行客户端。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

VERSIONS = {
    "1.20.1": (3465, "0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838", "bfn", "bfk"),
    "1.21.1": (3955, "30c73b1c5da787909b2f73340419fdf13b9def88", "bsx", "bsu"),
}


def f32(value):
    return struct.unpack(">f", struct.pack(">f", value))[0]


def constant(line, resolve=None):
    match = re.search(r"// float ([-+\deE.]+)f\s*$", line)
    if match:
        return f32(float(match[1]))
    match = re.search(r"\bfconst_([012])\s*$", line)
    if match:
        return float(match[1])
    match = re.search(r"// Field ([\w$]+)\.([\w$]+):F\s*$", line)
    if match and resolve:
        return resolve(match[1], match[2])
    raise ValueError(f"预期实际 Float 常量：{line}")


def instructions(segment):
    return [line for line in segment.splitlines() if re.match(r"\s*\d+:\s+[a-z]", line)]


def generate(version, client, output):
    data_version, expected, registry, dimensions = VERSIONS[version]
    raw = client.read_bytes()
    if hashlib.sha1(raw).hexdigest() != expected:
        raise ValueError("客户端与准确版本不符")
    sources = {}

    def read(name, verbose=False):
        options = ["-p", "-c", "-constants"] + (["-verbose"] if verbose else [])
        value = subprocess.check_output(["javap", "-classpath", str(client), *options, name], text=True)
        sources[name + (".verbose" if verbose else "")] = hashlib.sha256(value.encode()).hexdigest()
        return value

    constants = {}

    def static_float(owner, field):
        if owner not in constants:
            constants[owner] = read(owner)
        value = re.search(r"\bfloat " + re.escape(field) + r" = ([-+\deE.]+)f;", constants[owner])
        if value:
            return f32(float(value[1]))
        init = instructions(constants[owner].split("  static {};", 1)[1])
        writes = [i for i, line in enumerate(init) if re.search(r"putstatic\s+#\d+\s+// Field " + re.escape(field) + r":F\s*$", line)]
        if len(writes) != 1:
            raise ValueError(f"实体尺寸字段没有唯一静态赋值：{owner}.{field}")
        return constant(init[writes[0]-1], static_float)

    builder = registry + "$a"
    builder_code = read(builder)
    constructor = builder_code.split("private " + builder + "(", 1)[1].split("\n  public", 1)[0]
    calls = instructions(constructor)
    default = None
    for index, line in enumerate(calls):
        if f"Method {dimensions}.b:(FF)L{dimensions};" in line:
            default = [constant(calls[index-2]), constant(calls[index-1])]
    if default is None:
        raise ValueError("没有找到原生构造器的默认尺寸")
    fire = re.search(r"public " + re.escape(builder) + r"<T> (\w+)\(\);\s+Code:\s+0: aload_0\s+1: iconst_1\s+2: putfield\s+#\d+\s+// Field f:Z", builder_code)
    if fire is None:
        raise ValueError("没有找到实际耐火 builder")
    full = read(registry, True)
    init = full.split("  static {};", 1)[1].split("BootstrapMethods:", 1)[0]
    bootstraps = {}
    for match in re.finditer(r"^  (\d+): #[^\n]+\n(?:(?!^  \d+:).)*", full.split("BootstrapMethods:", 1)[1], re.M | re.S):
        constructor_class = re.search(r'REF_newInvokeSpecial ([\w$]+)\."<init>"', match[0])
        if constructor_class:
            bootstraps[int(match[1])] = constructor_class[1]
    entities = {}
    origins = {}
    previous = 0
    for match in re.finditer(r"putstatic\s+#\d+\s+// Field ([\w$]+):L" + registry + ";", init):
        segment = init[previous:match.end()]
        previous = match.end()
        names = re.findall(r"// String ([a-z0-9_:/.-]+)\s*$", segment, re.M)
        if len(names) != 1:
            raise ValueError(f"实体注册没有唯一 ID：{match[1]} {names}")
        name = names[0] if ":" in names[0] else "minecraft:" + names[0]
        width, height = default
        eye = None
        native_class = None
        calls = instructions(segment)
        for index, line in enumerate(calls):
            if f"Method {builder}.a:(FF)L{builder};" in line:
                width, height = constant(calls[index-2], static_float), constant(calls[index-1], static_float)
                eye = None
            if version == "1.21.1" and f"Method {builder}.b:(F)L{builder};" in line:
                eye = constant(calls[index-1])
            factory = re.search(r"// InvokeDynamic #(\d+):create:", line)
            if factory:
                native_class = bootstraps.get(int(factory[1]))
        if name in entities:
            raise ValueError(f"实体注册重复：{name}")
        entities[name] = {
            "width": width,
            "height": height,
            "eye": eye if eye is not None else f32(height * f32(0.85)),
            "fire_immune": f"Method {builder}.{fire[1]}:()L{builder};" in segment,
        }
        origins[name] = {"field": match[1], "class": native_class}
    if not entities:
        raise ValueError("没有实际实体注册")
    output.write_text(json.dumps({"data_version": data_version, "entities": entities}, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
    proof = {
        "source": "准确客户端静态 EntityType 注册、builder 与 bootstrap；未执行客户端或编译参照类",
        "client_sha256": hashlib.sha256(raw).hexdigest(),
        "javap_sha256": sources,
        "default_size": default,
        "origins": origins,
        "output_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
    }
    output.with_suffix(".proof.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{version}: {len(entities)} 个实际实体类型注册")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", choices=VERSIONS, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    generate(args.version, args.client, args.output)


if __name__ == "__main__":
    main()
