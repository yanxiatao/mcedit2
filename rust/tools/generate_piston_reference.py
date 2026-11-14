"""导出三版原生移动活塞渲染器的保存载荷、资源状态、位移及采样坐标。无窗口或反射。"""
import argparse
import hashlib
import gzip
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile
from client_reference import run_source_reference

SOURCE=r'''
import java.nio.file.*;import java.util.*;import com.google.gson.*;
public final class PistonRenderReference {
 static List<Object> parts;static double[] shift;
 @record@
 public static void main(String[] args)throws Exception {
  @bootstrap@
  @lookup@
  var rows=new ArrayList<Object>();
  @states@
  for(var moved:states)for(int facing=0;facing<6;facing++)for(boolean extending:new boolean[]{false,true})for(boolean source:new boolean[]{false,true}) {
   if(source&&!(@is_piston@))continue;
   for(float progress:new float[]{0f,0.24f,0.25f,0.26f,0.49f,0.5f,0.51f,0.75f,0.99999994f,1f,1.0000001f,4f,4.25f}) {
    var tag=new @compound@();@moved_tag@
    tag.a("id","minecraft:piston");tag.a("x",-19);tag.a("y",72);tag.a("z",-17);
    tag.a("facing",facing);tag.a("progress",progress);tag.a("extending",extending);tag.a("source",source);
    @entity@
    parts=new ArrayList<>();shift=new double[3];@render@
    var bytes=new java.io.ByteArrayOutputStream();tag.a(new java.io.DataOutputStream(bytes));
    rows.add(Map.of("nbt",bytes.toByteArray(),"parts",parts));
   }
  }
  Files.writeString(Path.of(args[0]),new Gson().toJson(rows)+"\n");
 }
}
'''
API={
 "1.21.1":dict(renderer="ghl",bootstrap="ab.a();akt.a();",lookup="jo.a lookup=ka.a(lt.aA);",compound="ub",
  states='var states=new ArrayList<dtc>();for(String id:List.of("air","stone","oak_stairs","piston","sticky_piston","piston_head","barrier","structure_void"))states.addAll(lt.e.a(akr.b(id)).l().a());',
  is_piston="moved.b() instanceof dsv",moved_tag='tag.a("blockState",uq.a(moved));',
  entity='var entity=new dsy(new jd(-19,72,-17),lt.e.a(akr.b("moving_piston")).o());entity.c(tag,lookup);',
  render='new ghl(null).a(entity,0.5f,new fbi(),null,0,0);',
  record='public static void record(jd pos,dtc state,fbi pose,boolean cull){var p=pose.c().a().getTranslation(new org.joml.Vector3f());parts.add(Map.of("state",state.toString().replace("Block{","").replace("}",""),"sample",new int[]{pos.u(),pos.v(),pos.w()},"translation",new float[]{p.x,p.y,p.z},"cull",cull));}'),
 "1.20.1":dict(renderer="fmg",bootstrap="aa.a();acs.a();",lookup="",compound="qr",
  states='var states=new ArrayList<dcb>();for(String id:List.of("air","stone","oak_stairs","piston","sticky_piston","piston_head","barrier","structure_void"))states.addAll(jb.f.a(new acq(id)).l().a());',
  is_piston="moved.b() instanceof dbu",moved_tag='tag.a("blockState",rd.a(moved));',
  entity='var entity=new dbx(new gu(-19,72,-17),jb.f.a(new acq("moving_piston")).n());entity.a(tag);',
  render='new fmg(null).a(entity,0.5f,new eij(),null,0,0);',
  record='public static void record(gu pos,dcb state,eij pose,boolean cull){var p=pose.c().a().getTranslation(new org.joml.Vector3f());parts.add(Map.of("state",state.toString().replace("Block{","").replace("}",""),"sample",new int[]{pos.u(),pos.v(),pos.w()},"translation",new float[]{p.x,p.y,p.z},"cull",cull));}'),
 "1.12.2":dict(renderer="bxd",bootstrap="ni.c();",lookup="",compound="fy",
  states='var states=new LinkedHashSet<awt>();for(int id:new int[]{0,1,53,29,33,34,166,217})for(var value:aow.c(id).s().a())states.add(aow.c(id).a(aow.c(id).e(value)));',
  is_piston="moved.u() instanceof awj",moved_tag='tag.a("blockId",aow.a(moved.u()));tag.a("blockData",moved.u().e(moved));',
  entity='var entity=new awm();entity.a(tag);',render='new bxd().a(entity,0.,0.,0.,0.5f,-1,1f);',
  record='public static void offset(buk ignored,double x,double y,double z){shift=new double[]{x,y,z};}public static boolean record(et pos,awt state,boolean cull){parts.add(Map.of("state",state.toString(),"sample",new int[]{pos.p(),pos.q(),pos.r()},"translation",new double[]{pos.p()+shift[0],pos.q()+shift[1],pos.r()+shift[2]},"cull",cull));return true;}'),
}

def model_reference(client,version,root):
 names={"moving_piston","piston","sticky_piston","piston_head","stone","oak_stairs","air","barrier","structure_void"}
 if version=="1.12.2":
  data=json.loads((root/"crates/mcedit2-world/assets/legacy-render/1.12.json").read_bytes())
  names.update(row[2] for row in data["entries"] if row[0] in {0,1,53,29,33,34,166,217} and row[2] is not None)
 documents={};queue=[]
 def references(value):
  if isinstance(value,dict):
   if isinstance(value.get("model"),str):queue.append(value["model"])
   for v in value.values():references(v)
  elif isinstance(value,list):
   for v in value:references(v)
 with zipfile.ZipFile(client) as jar:
  available=set(jar.namelist())
  for name in sorted(names):
   path="assets/minecraft/blockstates/"+name+".json"
   if path not in available:continue
   value=json.loads(jar.read(path));documents[path]=value;references(value)
  while queue:
   reference=queue.pop();ns,path=reference.split(":",1) if ":" in reference else ("minecraft",reference)
   if path.startswith("builtin/"):continue
   if "/" not in path:path="block/"+path
   path="assets/"+ns+"/models/"+path+".json"
   if path in documents:continue
   value=json.loads(jar.read(path));documents[path]=value
   if "parent" in value:queue.append(value["parent"])
 destination=root/"crates/mcedit2-world/tests/fixtures"/("piston-models-"+version+".json")
 destination.write_text(json.dumps(documents,sort_keys=True,separators=(",",":"))+"\n",encoding="utf-8")
 return hashlib.sha256(destination.read_bytes()).hexdigest()

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 for key in ("client-jar","version-json","libraries"):parser.add_argument("--"+key,type=Path,required=True)
 args=parser.parse_args();version=json.loads(args.version_json.read_text(encoding="utf-8"))["id"];api=API[version];root=Path(__file__).resolve().parents[1]
 with tempfile.TemporaryDirectory(prefix="piston-reference-",dir=root/"target") as temporary:
  work=Path(temporary);adapter=Path(__file__).with_name("PistonRenderAdapter.java")
  export="java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED"
  subprocess.run(["javac","--add-exports",export,"-encoding","UTF-8","-d",str(work),str(adapter)],check=True)
  def change(data):
   src=work/"renderer.class";dst=work/"adapted.class";src.write_bytes(data)
   subprocess.run(["java","--add-exports",export,"-cp",str(work),"PistonRenderAdapter",str(src),str(dst),version],check=True)
   return dst.read_bytes()
  source=SOURCE
  for key,value in api.items():source=source.replace("@"+key+"@",value)
  args.output=work/"reference.json"
  proof=run_source_reference(args,"PistonRenderReference",source,work,version,{api["renderer"]+".class":change})
  raw=args.output.read_bytes()
  destination=root/"crates/mcedit2-world/tests/fixtures"/("piston-render-"+version+".json.gz");destination.write_bytes(gzip.compress(raw,mtime=0))
  proof.update(source_sha256=hashlib.sha256(source.encode()).hexdigest(),adapter_sha256=hashlib.sha256(adapter.read_bytes()).hexdigest(),scope="原生渲染器分支与状态 / PoseStack / 采样坐标；构造和实际模型提交边界被截获；不包含 GPU、AO、照明或原生模型像素")
  proof["compressed_sha256"]=hashlib.sha256(destination.read_bytes()).hexdigest()
  proof["model_documents_sha256"]=model_reference(args.client_jar,version,root)
  destination.with_name("piston-render-"+version+".proof.json").write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
  print(version,len(json.loads(raw)),"组原生绘制记录")

if __name__=="__main__":main()
