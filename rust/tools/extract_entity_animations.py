"""静态读取准确客户端关键帧常量的数据流，不加载或执行客户端类。"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
import zipfile

VERSIONS = {
    "1.20.1": (3465, "56b71336d2b4fdffd197f56595b0da93e32a946f78f382a299b8f4b92758bb0f"),
    "1.21.1": (3955, "499f6897d1837516680f3114072d8106e11c9adcd933fe5cf051b551089b0c99"),
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def f32(value):
    return struct.unpack(">f", struct.pack(">f", value))[0]


class Constant:
    """只记录常量对象的别名与构造参数；不创建对应 Java 对象。"""
    def __init__(self, kind, value=None):
        self.kind, self.value = kind, value


def unwrap(value):
    if isinstance(value, Constant):
        return unwrap(value.value)
    if isinstance(value, list):
        return [unwrap(item) for item in value]
    if isinstance(value, dict):
        return {key: unwrap(item) for key, item in value.items()}
    return value


def classes(text):
    return {m[1]: (m[2], m[3]) for m in re.finditer(
        r"^([\w.$]+) -> ([\w$]+):\n(.*?)(?=^[\w.$]+ -> |\Z)", text, re.M | re.S
    )}


def descriptors(signature):
    arguments = signature[1:signature.index(")")]
    return re.findall(r"\[*(?:L[^;]+;|[BCDFIJSZ])", arguments)


def static_constants(code, owner, symbols, members):
    stack, result = [], {}
    for row in code.splitlines():
        instruction = re.match(r"\s*(\d+):\s+(\w+)\s*(.*)", row)
        if not instruction:
            continue
        pc, op, operand = instruction.groups()
        try:
            if op in ("ldc", "ldc_w", "ldc2_w"):
                kind, value = operand.split("// ", 1)[1].split(" ", 1)
                if kind == "String": stack.append(value)
                elif kind == "float": stack.append(f32(float(value.removesuffix("f"))))
                elif kind == "double": stack.append(float(value.removesuffix("d")))
                elif kind in ("int", "long"): stack.append(int(value.removesuffix("l")))
                else: raise ValueError(f"不是受支持的常量：{kind}")
            elif op.startswith(("fconst_", "dconst_", "iconst_")):
                value = op.split("_")[1]
                number = -1 if value == "m1" else int(value)
                stack.append(number if op.startswith("iconst_") else float(number))
            elif op in ("bipush", "sipush"):
                stack.append(int(operand))
            elif op == "new":
                stack.append(Constant(operand.split("// class ")[1]))
            elif op == "dup":
                stack.append(stack[-1])
            elif op == "anewarray":
                stack.append([None] * stack.pop())
            elif op == "aastore":
                value, index, array = stack.pop(), stack.pop(), stack.pop()
                array[index] = value
            elif op in ("getstatic", "putstatic"):
                field = operand.split("// Field ")[1].split(":")[0]
                cls, name = field.rsplit(".", 1) if "." in field else (owner, field)
                if op == "getstatic":
                    stack.append(symbols[(cls, name)])
                else:
                    member = members[name]
                    result[member] = unwrap(stack.pop())
            elif op in ("invokestatic", "invokevirtual", "invokespecial"):
                method = operand.split("// Method ")[1]
                target, signature = method.split(":", 1)
                cls, name = target.rsplit(".", 1)
                name = name.strip('"')
                count = len(descriptors(signature))
                args = stack[-count:] if count else []
                if count: del stack[-count:]
                receiver = stack.pop() if op != "invokestatic" else None
                if cls == symbols["builder"]:
                    if signature.startswith("(F)") and op == "invokestatic":
                        stack.append(Constant(cls, {"length": args[0], "looping": False, "bones": {}}))
                    elif signature.startswith("()") and signature.endswith("L" + cls + ";"):
                        receiver.value["looping"] = True
                        stack.append(receiver)
                    elif signature.startswith("(Ljava/lang/String;"):
                        receiver.value["bones"].setdefault(args[0], []).append(args[1])
                        stack.append(receiver)
                    elif signature.startswith("()") and signature.endswith("L" + symbols["definition"] + ";"):
                        stack.append(receiver)
                    else: raise ValueError(f"未知动画 Builder 调用：{method}")
                elif cls == symbols["vectors"]:
                    values = args
                    operation = symbols[("vector", name, signature)]
                    if operation == "position": values = [values[0], -values[1], values[2]]
                    elif operation == "rotation": values = [f32(value * f32(0.017453292)) for value in values]
                    elif operation == "scale": values = [f32(value - 1.) for value in values]
                    stack.append(values)
                elif name == "<init>" and cls == symbols["keyframe"]:
                    receiver.value = {"time": args[0], "value": args[1], "interpolation": args[2]}
                elif name == "<init>" and cls == symbols["channel"]:
                    receiver.value = {"target": args[0], "keyframes": args[1]}
                elif name == "<init>" and cls == "org/joml/Vector3f":
                    receiver.value = args
                else: raise ValueError(f"未知关键帧常量调用：{method}")
            elif op == "return":
                if stack: raise ValueError("常量数据流结束后仍有未消费的操作数")
            else:
                raise ValueError(f"不是关键帧常量构造指令：{op}")
        except (KeyError, IndexError, ValueError, TypeError) as error:
            raise ValueError(f"{owner} 静态常量 {pc}: {op} {operand}：{error}") from error
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, choices=VERSIONS)
    parser.add_argument("--client", required=True, type=Path)
    parser.add_argument("--mappings", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    version, expected = VERSIONS[args.version]
    if sha(args.client.read_bytes()) != expected:
        raise ValueError("客户端摘要与指定版本不一致")
    mapping_bytes = args.mappings.read_bytes()
    mapping = classes(mapping_bytes.decode("utf8"))
    prefix = "net.minecraft.client.animation."
    symbols = {key: mapping[prefix + name][0] for key, name in [
        ("builder", "AnimationDefinition$Builder"), ("definition", "AnimationDefinition"),
        ("vectors", "KeyframeAnimations"), ("keyframe", "Keyframe"), ("channel", "AnimationChannel"),
    ]}
    for name, values in [("AnimationChannel$Targets", {"POSITION": "position", "ROTATION": "rotation", "SCALE": "scale"}),
                         ("AnimationChannel$Interpolations", {"LINEAR": "linear", "CATMULLROM": "catmull_rom"})]:
        cls, body = mapping[prefix + name]
        for semantic, encoded in values.items():
            field = re.search(r"\s" + semantic + r" -> (\w+)$", body, re.M)[1]
            symbols[(cls, field)] = encoded
    for semantic, name, signature in [("position", "posVec", "(FFF)Lorg/joml/Vector3f;"),
                                      ("rotation", "degreeVec", "(FFF)Lorg/joml/Vector3f;"),
                                      ("scale", "scaleVec", "(DDD)Lorg/joml/Vector3f;")]:
        method = re.search(r"\s" + name + r"\(.*? -> (\w+)$", mapping[prefix + "KeyframeAnimations"][1], re.M)[1]
        symbols[("vector", method, signature)] = semantic
    definitions, entries = {}, {}
    with zipfile.ZipFile(args.client) as archive:
        for name, (cls, body) in mapping.items():
            if not name.startswith(prefix + "definitions.") or "$" in name:
                continue
            fields = {m[2]: m[1] for m in re.finditer(r"net.minecraft.client.animation.AnimationDefinition (\w+) -> (\w+)", body)}
            if not fields: continue
            entry = cls + ".class"
            entries[entry] = sha(archive.read(entry))
            disassembly = subprocess.check_output(["javap", "-p", "-c", "-classpath", str(args.client), cls], text=True)
            initializer = next(part for part in re.split(r"\n(?=  (?:public|private|protected|static) )", disassembly)
                               if part.strip().startswith("static {};"))
            actor = name.rsplit(".", 1)[1].removesuffix("Animation").lower()
            for member, definition in static_constants(initializer, cls, symbols, fields).items():
                key = member.removeprefix(actor.upper() + "_").lower()
                definitions["minecraft:" + actor + "/" + key] = definition
        for key in symbols:
            if isinstance(key, str):
                entry = symbols[key] + ".class"
                entries[entry] = sha(archive.read(entry))
        for name in ["AnimationChannel$Targets", "AnimationChannel$Interpolations"]:
            entry = mapping[prefix + name][0] + ".class"
            entries[entry] = sha(archive.read(entry))
    output = (json.dumps({"data_version": version, "definitions": definitions}, ensure_ascii=False, indent=2) + "\n").encode("utf8")
    args.output.write_bytes(output)
    args.output.with_suffix(".proof.json").write_text(json.dumps({
        "version": args.version, "client_sha256": expected, "mappings_sha256": sha(mapping_bytes),
        "entries_sha256": entries, "extractor_sha256": sha(Path(__file__).read_bytes()),
        "output_sha256": sha(output), "scope": "仅静态读取 class 常量构造数据流；不加载或执行客户端类，无编译或测试",
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf8")
    print(f"静态读取 {len(definitions)} 个动画定义")


if __name__ == "__main__":
    main()
