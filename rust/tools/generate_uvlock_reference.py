"""用用户已有的官方 1.21.1 客户端和本机 JDK 生成 UV 数值参照，不下载或修改启动器。"""
import argparse
import json
import pathlib
from client_reference import run_reference


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("client-jar", "version-json", "libraries", "output"):
        parser.add_argument("--" + name, required=True, type=pathlib.Path)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    work = root / "target/uvlock-reference"
    evidence = run_reference(args, "UvLockReference", work)
    evidence["case_count"] = len(json.loads(args.output.read_text(encoding="utf-8"))["cases"])
    (work / "evidence.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8", newline="\n")
    print(json.dumps(evidence, ensure_ascii=False))


if __name__ == "__main__":
    main()
