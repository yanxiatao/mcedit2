"""从准确客户端的公开模型接口生成可校验的模型层数据；不启动游戏。"""
import argparse
import json
import pathlib
import hashlib
from client_reference import run_source_reference
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client-jar", type=pathlib.Path, required=True)
    parser.add_argument("--version-json", type=pathlib.Path, required=True)
    parser.add_argument("--libraries", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--reference-output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    version=json.loads(args.version_json.read_text(encoding="utf-8"))["id"]
    source=pathlib.Path(__file__).with_name("EntityLayerReference.java").read_text(encoding="utf-8")
    if version=="1.20.1":
        begin=source.index("    private static final class Sink")
        end=source.index("    private static fyk node",begin)
        source=source[:begin]+"""    private static final class Sink implements ein {
        final List<float[]> vertices=new ArrayList<>();
        float[] vertex;
        public ein a(double x,double y,double z){vertex=new float[]{(float)x,(float)y,(float)z,0,0};return this;}
        public ein a(int r,int g,int b,int a){return this;}
        public ein a(float u,float v){vertex[3]=u;vertex[4]=v;return this;}
        public ein a(int u,int v){return this;}
        public ein b(int u,int v){return this;}
        public ein a(float x,float y,float z){return this;}
        public void e(){vertices.add(vertex);vertex=null;}
        public void b(int r,int g,int b,int a){throw new UnsupportedOperationException("本参照不使用默认颜色");}
        public void k(){throw new UnsupportedOperationException("本参照不使用默认颜色");}
    }
"""+source[end:]
        for before,after in [("fyk","fee"),("fbi","eij"),("fyh","feb"),("ab.a(); akt.a();","aa.a(); acs.a();"),(",0,0,-1)",",0,0,1,1,1,1)"),("1.21.1","1.20.1")]:source=source.replace(before,after)
    elif version!="1.21.1":raise ValueError("该客户端模型 API 未绑定")
    base=pathlib.Path(__file__).resolve().parents[1]/"target"
    runtime_output=args.output.resolve()
    with tempfile.TemporaryDirectory(prefix="entity-layers-",dir=base) as directory:
        work=pathlib.Path(directory)
        args.output=work/"all-entity-layers.json"
        proof=run_source_reference(args,"EntityLayerReference",source,work,version)
        data=json.loads(args.output.read_text(encoding="utf-8"))
    runtime={"version":data["version"],"layers":data["layers"]}
    reference={"version":data["version"],"reference":data["reference"],"poses":data["poses"],"living":data["living"]}
    if version=="1.20.1":
        canonical=lambda value:json.dumps(value,sort_keys=True,separators=(",",":"))
        base_catalog=runtime_output.with_name("entity-layers-1.21.1.json")
        base_reference=base/"../crates/mcedit2-world/tests/fixtures/entity-layer-reference-1.21.1.json"
        old=json.loads(base_catalog.read_text(encoding="utf-8"));old_ref=json.loads(base_reference.read_text(encoding="utf-8"))
        shared=[key for key,value in data["layers"].items()if key in old["layers"]and canonical(value)==canonical(old["layers"][key])]
        shared_ref=[key for key,value in data["reference"].items()if key in old_ref["reference"]and canonical(value)==canonical(old_ref["reference"][key])]
        runtime["shared_layers"]=shared;runtime["layers"]={key:value for key,value in data["layers"].items()if key not in shared}
        reference["shared_reference"]=shared_ref;reference["reference"]={key:value for key,value in data["reference"].items()if key not in shared_ref}
        proof["shared_catalog_sha256"]=hashlib.sha256(base_catalog.read_bytes()).hexdigest()
        proof["shared_reference_sha256"]=hashlib.sha256(base_reference.read_bytes()).hexdigest()
        proof["shared_layers"]=len(shared);proof["shared_reference_layers"]=len(shared_ref)
    runtime_output.parent.mkdir(parents=True,exist_ok=True)
    runtime_output.write_text(json.dumps(runtime,separators=(",",":"))+"\n",encoding="utf-8",newline="\n")
    args.reference_output.parent.mkdir(parents=True,exist_ok=True)
    args.reference_output.write_text(json.dumps(reference,separators=(",",":"))+"\n",encoding="utf-8",newline="\n")
    proof["catalog_sha256"]=hashlib.sha256(runtime_output.read_bytes()).hexdigest()
    proof["reference_sha256"]=hashlib.sha256(args.reference_output.read_bytes()).hexdigest()
    proof["layers"]=len(data["layers"])
    runtime_output.with_suffix(".proof.json").write_text(
        json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
