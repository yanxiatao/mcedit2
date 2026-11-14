"""从仓库中原版 BlockDumper 数据生成只用于渲染的数字 ID 绑定，不改写世界状态。"""
import argparse
import json
from pathlib import Path
import subprocess


def generate(reference, output):
    output.mkdir(parents=True, exist_ok=True)
    for version, suffix in [("1.8", ""), ("1.9", "_19"), ("1.10", "_1_10"),
                            ("1.11", "_1_11"), ("1.12", "_1_12")]:
        def read(name):
            path = f"{reference}:src/mceditlib/blocktypes/{name}{suffix}.json"
            return json.loads(subprocess.check_output(["git", "show", path]))
        states = {item["blockState"]: item for item in read("minecraft_raw")}
        entries = []
        seen = set()
        for block_id, metadata, state in read("idmapping_raw"):
            if (block_id, metadata) in seen:
                raise ValueError(f"重复数字状态：{version} {block_id}:{metadata}")
            seen.add((block_id, metadata))
            source = states[state]
            entries.append([block_id, metadata, source.get("resourcePath"),
                            source.get("resourceVariant"), source["internalName"], state])
        content = {"source": reference, "version": version, "entries": entries}
        (output / f"{version}.json").write_text(
            json.dumps(content, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")
        print(f"{version}: {len(entries)} 个原版数字状态")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    generate(arguments.reference, arguments.output)
