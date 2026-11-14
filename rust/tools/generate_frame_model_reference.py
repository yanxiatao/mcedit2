"""从准确原版客户端读取展示框模型及父链，供既有渲染类别集中验证。"""
import argparse
import hashlib
import json
from pathlib import Path
import zipfile

CLIENTS = {
    "1.12.2": (1343, "0f275bc1547d01fa5f56ba34bdc87d981ee12daf"),
    "1.20.1": (3465, "0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838"),
    "1.21.1": (3955, "30c73b1c5da787909b2f73340419fdf13b9def88"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("client_root", type=Path)
    args = parser.parse_args()
    result = {}
    for version, (data_version, expected) in CLIENTS.items():
        path = args.client_root / version / f"minecraft-{version}-client.jar"
        sha1 = hashlib.sha1(path.read_bytes()).hexdigest()
        if sha1 != expected:
            raise ValueError(f"{version} 客户端 SHA1 不一致：{sha1}")
        with zipfile.ZipFile(path) as archive:
            queue = [f"block/{glow}item_frame{suffix}" for glow in
                     ([""] if data_version == 1343 else ["", "glow_"])
                     for suffix in ["", "_map"]]
            documents = {}
            while queue:
                model = queue.pop()
                name = f"assets/minecraft/models/{model}.json"
                if name in documents:
                    continue
                document = json.loads(archive.read(name))
                documents[name] = document
                if "parent" in document:
                    queue.append(document["parent"].removeprefix("minecraft:"))
            result[str(data_version)] = {"version": version, "sha1": sha1, "models": documents}
    root = Path(__file__).resolve().parents[1]
    output = root / "crates/mcedit2-iced/tests/fixtures/frame-models.json"
    output.write_text(json.dumps(result, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
