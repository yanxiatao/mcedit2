"""执行官方 1.21.1 公开模型 renderToBuffer，生成静态 NPC 数值参考。"""
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
    work = pathlib.Path(__file__).resolve().parents[1] / "target/npc-model-reference"
    proof = run_reference(args, "NpcModelReference", work)
    source = pathlib.Path(__file__).with_name("NpcModelReference.java")
    result = json.loads(args.output.read_text(encoding="utf-8"))
    proof.update({
        "client_sha1": "30c73b1c5da787909b2f73340419fdf13b9def88",
        "java_source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "case_count": len(result["cases"]),
        "executed": "官方客户端公开模型构造、模型层烘焙、ModelPart 字段、PoseStack 和模型 renderToBuffer",
        "manually_applied": "静态字节码确认的 living 外层变换顺序、年龄标志、部件姿态与可见性；未调用实体 setupAnim 或完整 renderer",
        "limitations": "无世界、实体、实际游戏客户端窗口、纹理像素、装备、动态动画、观察者与完整 renderer。",
    })
    args.output.with_suffix(".proof.json").write_text(json.dumps(proof, ensure_ascii=False, indent=2)+"\n",encoding="utf-8")


if __name__ == "__main__":
    main()
