"""使用官方公开属性 API 生成 SCALE 值与实际持久 NBT 参照。"""
import argparse
import hashlib
import json
import pathlib
from client_reference import run_reference


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-jar", type=pathlib.Path, required=True)
    parser.add_argument("--version-json", type=pathlib.Path, required=True)
    parser.add_argument("--libraries", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    work = pathlib.Path(__file__).resolve().parents[1] / "target/scale-attribute-reference"
    proof = run_reference(args, "ScaleAttributeReference", work)
    source = pathlib.Path(__file__).with_name("ScaleAttributeReference.java")
    result = json.loads(args.output.read_text(encoding="utf-8"))
    proof.update({"client_sha1":"30c73b1c5da787909b2f73340419fdf13b9def88",
        "java_source_sha256":hashlib.sha256(source.read_bytes()).hexdigest(),
        "case_count":len(result["cases"]),
        "executed":"官方注册 SCALE、AttributeInstance、持久修饰符、getValue、save 和 NbtIo 二进制输出；变更回调实际计数",
        "limitations":"无实体、世界、装备动态修饰符或游戏客户端渲染。"})
    args.output.with_suffix(".proof.json").write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")


if __name__ == "__main__":
    main()
