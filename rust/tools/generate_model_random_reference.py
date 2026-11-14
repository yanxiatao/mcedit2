"""生成官方 1.21.1 的加权模型选择与双格方块种子参照。"""
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
    work = root / "target/model-random-reference"
    evidence = run_reference(args, "ModelRandomReference", work)
    result = json.loads(args.output.read_text(encoding="utf-8"))
    evidence.update(random_cases=len(result["random"]), block_seed_cases=len(result["block_seeds"]))
    (work / "evidence.json").write_text(json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8", newline="\n")
    print(json.dumps(evidence, ensure_ascii=False))


if __name__ == "__main__":
    main()
