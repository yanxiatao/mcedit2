"""静态提取准确客户端的全部物品标签；不编译或执行客户端。"""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

CLIENTS = {
    "1.20.1": ("tags/items", "56b71336d2b4fdffd197f56595b0da93e32a946f78f382a299b8f4b92758bb0f"),
    "1.21.1": ("tags/item", "499f6897d1837516680f3114072d8106e11c9adcd933fe5cf051b551089b0c99"),
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", choices=CLIENTS, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    registry, expected = CLIENTS[args.version]
    if sha(args.client.read_bytes()) != expected:
        raise ValueError("客户端摘要与指定版本不一致")
    prefix = "data/minecraft/" + registry + "/"
    tags, entries = {}, {}
    with zipfile.ZipFile(args.client) as archive:
        for name in sorted(archive.namelist()):
            if name.startswith(prefix) and name.endswith(".json"):
                source = archive.read(name)
                tags["minecraft:" + name[len(prefix):-5]] = json.loads(source)
                entries[name] = sha(source)
    if "minecraft:piglin_loved" not in tags:
        raise ValueError("准确客户端没有原生 piglin_loved 标签")
    output = (json.dumps(tags, ensure_ascii=False, indent=2) + "\n").encode("utf8")
    args.output.write_bytes(output)
    proof = {
        "version": args.version, "client_sha256": expected,
        "registry": registry, "entries_sha256": entries,
        "extractor_sha256": sha(Path(__file__).read_bytes()),
        "output_sha256": sha(output),
        "scope": "仅静态读取客户端归档中的原始 JSON 标签；无编译、客户端执行或测试",
    }
    args.output.with_suffix(".proof.json").write_text(
        json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf8"
    )


if __name__ == "__main__":
    main()
