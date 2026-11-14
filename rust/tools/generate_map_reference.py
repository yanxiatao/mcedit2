"""从准确客户端公共地图接口导出标记目录、持久化载荷及实际投影。无反射或窗口。"""
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import subprocess
from client_reference import run_source_reference

TOOLS = Path(__file__).resolve().parent

SOURCE = '''import java.nio.file.*;import java.util.*;import com.google.gson.*;
public class MapReference {
 public static eqt current;static List<float[]> vertices;
 public static gql sprite(gri textures,eqn marker){return null;}
 static byte[] payload(ub tag)throws Exception {var bytes=new java.io.ByteArrayOutputStream();tag.a(new java.io.DataOutputStream(bytes));return bytes.toByteArray();}
 static class Sink implements fbm {
  float[] point;
  public fbm a(float x,float y,float z){point=new float[]{x,y,z,0,0};vertices.add(point);return this;}
  public fbm a(float u,float v){point[3]=u;point[4]=v;return this;}
  public fbm a(int r,int g,int b,int a){return this;} public fbm a(int u,int v){return this;}public fbm b(int u,int v){return this;}
  public fbm b(float x,float y,float z){return this;}
 }
 public static void main(String[] args)throws Exception {
 ab.a();akt.a();var lookup=ka.a(lt.aA);
 var types=new TreeMap<String,Object>();
 for(var type:lt.at){types.put(lt.at.b(type).toString(),Map.of("asset_id",type.b().toString(),"show_on_frame",type.c()));}
 var banners=new ArrayList<Object>();
 for(var dye:cti.values()){
  var banner=new eqm(new jd(-19,72,-17),dye,Optional.of(wz.b("Marker")));
  var tag=eqm.a.encodeStart(lookup.a(up.a),banner).getOrThrow();
  banners.add(Map.of("nbt",tag.toString(),"payload",payload((ub)tag),"type",lt.at.b(banner.a().a()).toString()));
 }
 var samples=new ArrayList<Object>();
 for(int scale=0;scale<5;scale++)for(int x:new int[]{-2048,-64,-63,-1,0,1,63,64,2048}){
  var tag=uz.a("{dimension:\\"minecraft:overworld\\",xCenter:0,zCenter:0,scale:"+scale+"b,banners:[],frames:[]}");
  var frames=new uh();frames.add(new eqq(new jd(x,72,-x),90,1).a());tag.a("frames",frames);
  var saved=eqt.b(tag,lookup);var decorations=new ArrayList<Object>();
  for(var marker:saved.h())decorations.add(Map.of("type",lt.at.b(marker.c().a()).toString(),"x",marker.d(),"y",marker.e(),"rotation",marker.f()));
  samples.add(Map.of("scale",scale,"position",new int[]{x,72,-x},"payload",payload(tag),"frame_nbt",frames.toString(),"decorations",decorations));
 }
 var projections=new ArrayList<Object>();
 for(var type:List.of(eqp.a,eqp.b,eqp.e))for(int scale=0;scale<5;scale++)for(int x:new int[]{-2048,-320,-64,-63,-1,0,1,63,64,320,2048})for(boolean unlimited:new boolean[]{false,true}){
  var tag=uz.a("{dimension:\\"minecraft:overworld\\",xCenter:0,zCenter:0,scale:"+scale+"b,banners:[],frames:[]}");tag.a("unlimitedTracking",unlimited);
  var data=eqt.b(tag,lookup);eqt.project(data,type,null,"sample",x,-x,90.,null);
  var markers=new ArrayList<Object>();for(var m:data.h())markers.add(Map.of("type",lt.at.b(m.c().a()).toString(),"x",m.d(),"y",m.e(),"rotation",m.f()));
  projections.add(Map.of("type",lt.at.b(type.a()).toString(),"scale",scale,"x",x,"z",-x,"unlimited",unlimited,"decorations",markers));
 }
 var geometry=new ArrayList<Object>();
 for(var type:lt.at)for(byte rot=0;rot<16;rot++)for(boolean onFrame:new boolean[]{false,true}){
  current=eqt.a((byte)0,true,dcw.h);current.a(List.of(new eqn(jm.a(type),(byte)-17,(byte)39,rot,Optional.empty())));
  vertices=new ArrayList<>();fic.a.a((fic.a)null,new fbi(),renderType->new Sink(),onFrame,15728880);
  geometry.add(Map.of("type",lt.at.b(type).toString(),"rotation",rot,"on_frame",onFrame,"vertices",vertices));
 }
 Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("types",types,"banners",banners,"samples",samples,"geometry",geometry,"projections",projections)));
 }}
'''

SOURCE_120 = '''import java.nio.file.*;import java.util.*;import com.google.gson.*;
public class MapReference {public static void main(String[] args)throws Exception {
 aa.a();acs.a();var types=new TreeMap<String,Object>();var banners=new ArrayList<Object>();var samples=new ArrayList<Object>();
 for(var type:dyl.a.values())types.put("minecraft:"+type.name().toLowerCase(Locale.ROOT),Map.of("legacy_icon",type.a(),"show_on_frame",type.b()));
 for(var dye:cen.values()){var banner=new dyk(new gu(-19,72,-17),dye,sw.b("Marker"));var bytes=new java.io.ByteArrayOutputStream();banner.e().a(new java.io.DataOutputStream(bytes));banners.add(Map.of("nbt",banner.e().toString(),"payload",bytes.toByteArray(),"type","minecraft:"+banner.c().name().toLowerCase(Locale.ROOT)));}
 for(int scale=0;scale<5;scale++)for(int x:new int[]{-2048,-64,-63,-1,0,1,63,64,2048}){
  var tag=new qr();tag.a("dimension","minecraft:overworld");tag.a("scale",(byte)scale);var frames=new qx();frames.add(new dym(new gu(x,72,-x),90,1).a());tag.a("frames",frames);
  var saved=dyo.b(tag);var markers=new ArrayList<Object>();for(var m:saved.f())markers.add(Map.of("type","minecraft:"+m.b().name().toLowerCase(Locale.ROOT),"x",m.c(),"y",m.d(),"rotation",m.e()));
  var bytes=new java.io.ByteArrayOutputStream();tag.a(new java.io.DataOutputStream(bytes));samples.add(Map.of("scale",scale,"position",new int[]{x,72,-x},"payload",bytes.toByteArray(),"frame_nbt",frames.toString(),"decorations",markers));
 }
 Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("types",types,"banners",banners,"samples",samples)));
}}
'''
SOURCE_112 = '''import java.nio.file.*;import java.util.*;import com.google.gson.*;
public class MapReference {public static void main(String[] args)throws Exception {
 ni.c();var types=new TreeMap<String,Object>();for(var type:beu.a.values())types.put("minecraft:"+type.name().toLowerCase(Locale.ROOT),Map.of("legacy_icon",type.a(),"show_on_frame",type.b()));
 Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("types",types,"banners",List.of(),"samples",List.of())));
}}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ["client-jar", "version-json", "libraries"]:
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    base = TOOLS.parent / "target/map-reference"
    base.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cpu-", dir=base) as directory:
        work = Path(directory)
        version = json.loads(args.version_json.read_text(encoding="utf-8"))
        if "id" not in version:
            version["id"] = version["version"]
            for lib in version["libraries"]:
                artifact = lib.get("downloads", {}).get("artifact", {})
                group, name, number = lib["name"].split(":")[:3]
                artifact["path"] = f"{group.replace('.', '/')}/{name}/{number}/{name}-{number}.jar"
            args.version_json = work / "version.json"
            args.version_json.write_text(json.dumps(version), encoding="utf-8")
        version = version["id"]
        args.output = work / "reference.json"
        source = {"1.21.1": SOURCE, "1.20.1": SOURCE_120, "1.12.2": SOURCE_112}[version]
        adapters = None
        if version == "1.21.1":
            subprocess.run(["javac", "--add-exports", "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED", "-d", str(work), str(TOOLS/"MapCpuAdapter.java")], check=True)
            def adapt(data):
                original=work/"original.class";changed=work/"adapted.class"
                original.write_bytes(data)
                subprocess.run(["java", "--add-exports", "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED", "-cp", str(work), "MapCpuAdapter", str(original), str(changed)], check=True)
                return changed.read_bytes()
            adapters = {"fic$a.class": adapt, "fic.class": adapt, "eqt.class": adapt}
        proof = run_source_reference(args, "MapReference", source, work, version, adapters)
        if adapters:
            proof["adapter_source_sha256"] = hashlib.sha256((TOOLS/"MapCpuAdapter.java").read_bytes()).hexdigest()
        proof["generator_source_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        target = TOOLS.parent / f"crates/mcedit2-world/tests/fixtures/map-reference-{version}.json"
        target.write_bytes(args.output.read_bytes())
        target.with_suffix(".proof.json").write_text(json.dumps(proof, indent=2) + "\n", encoding="utf-8")
        types = json.loads(args.output.read_text())["types"]
        (TOOLS.parent / f"crates/mcedit2-world/assets/map-decoration-types-{version}.json").write_text(
            json.dumps(types, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
