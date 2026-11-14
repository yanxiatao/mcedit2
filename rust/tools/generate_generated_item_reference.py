"""调用准确 1.21.1 ItemModelGenerator，导出静态 / 动画 / 镂空帧的原生元素与 UV。"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from client_reference import run_source_reference

TOOLS = Path(__file__).resolve().parent
CASES = [
    {"name":"solid","width":1,"height":1,"alpha":[[255]],"indices":[0]},
    {"name":"empty","width":3,"height":2,"alpha":[[0]*6],"indices":[0]},
    {"name":"ring","width":5,"height":3,"alpha":[[255]*5+[255,0,0,0,255]+[255]*5],"indices":[0]},
    {"name":"gaps","width":7,"height":2,"alpha":[[255,0,0,255,0,0,255,0,0,255,0,255,0,0]],"indices":[0]},
    {"name":"nonbinary","width":3,"height":2,"alpha":[[1,0,128,255,64,0]],"indices":[0]},
    {"name":"animated","width":3,"height":2,"alpha":[[255,0,0,0,0,0],[0,255,0,0,255,0],[0,0,255,255,0,0]],"indices":[0,1,2]},
    {"name":"unused","width":3,"height":2,"alpha":[[255,0,0,0,0,0],[255]*6,[0,0,0,0,0,255]],"indices":[0,2]},
]

SOURCE = '''import java.nio.file.*;import java.util.*;import java.util.stream.*;import com.google.gson.*;
public class GeneratedItemReference {
 static JsonObject current;
 public static int width(gqf sprite){return current.get("width").getAsInt();}
 public static int height(gqf sprite){return current.get("height").getAsInt();}
 public static IntStream frames(gqf sprite){return StreamSupport.stream(current.getAsJsonArray("indices").spliterator(),false).mapToInt(JsonElement::getAsInt);}
 public static boolean transparent(gqf sprite,int frame,int x,int y){return current.getAsJsonArray("alpha").get(frame).getAsJsonArray().get(y*width(sprite)+x).getAsInt()==0;}
 public static float shrink(gql sprite){return 0;}
 public static float uv(gql sprite,float value){return value;}
 public static void main(String[] args)throws Exception {
 var records=new ArrayList<Object>();
 for(var input:JsonParser.parseString(%s).getAsJsonArray()){
  current=input.getAsJsonObject();var elements=new ArrayList<Object>();var quads=new ArrayList<Object>();
  for(var raw:gge.a((gge)null,3,"#layer3",(gqf)null)){
   var element=(gfx)raw;
   var faces=new TreeMap<String,Object>();
   for(var e:element.c.entrySet()){
    var f=e.getValue();faces.put(e.getKey().name().toLowerCase(Locale.ROOT),Map.of("texture",f.c(),"tintindex",f.b(),"uv",f.d().a));
    var baked=ggd.a((ggd)null,element.a,element.b,f,(gql)null,e.getKey(),gsn.a(0,0),null,true);
    var data=baked.b();if(data.length!=32)throw new IllegalStateException("原版 BakedQuad 顶点格式改变");
    var vertices=new ArrayList<float[]>();for(int i=0;i<4;i++){
     int at=i*8;vertices.add(new float[]{Float.intBitsToFloat(data[at])-0.5f,Float.intBitsToFloat(data[at+1])-0.5f,Float.intBitsToFloat(data[at+2])-0.5f,Float.intBitsToFloat(data[at+4]),Float.intBitsToFloat(data[at+5])});
    }
    quads.add(Map.of("vertices",vertices,"normal",baked.e().name().toLowerCase(Locale.ROOT),"tint",baked.d()));
   }
   elements.add(Map.of("from",new float[]{element.a.x(),element.a.y(),element.a.z()},"to",new float[]{element.b.x(),element.b.y(),element.b.z()},"faces",faces));
  }
  current.add("elements",new Gson().toJsonTree(elements));current.add("quads",new Gson().toJsonTree(quads));records.add(current);
 }
 Files.writeString(Path.of(args[0]),new Gson().toJson(records));
 }}
'''

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ["client-jar","version-json","libraries"]:
        parser.add_argument("--"+key,type=Path,required=True)
    args=parser.parse_args()
    root=TOOLS.parent
    base=root/"target/generated-item-reference"
    base.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="cpu-",dir=base) as temp:
        work=Path(temp)
        subprocess.run(["javac","--add-exports","java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED","-d",str(work),str(TOOLS/"GeneratedItemCpuAdapter.java")],check=True)
        def adapt(data):
            original=work/"original.class";changed=work/"adapted.class"
            original.write_bytes(data)
            subprocess.run(["java","--add-exports","java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED","-cp",str(work),"GeneratedItemCpuAdapter",str(original),str(changed)],check=True)
            return changed.read_bytes()
        args.output=work/"reference.json"
        code=SOURCE % json.dumps(json.dumps(CASES,separators=(",",":")))
        proof=run_source_reference(args,"GeneratedItemReference",code,work,"1.21.1",{"gge.class":adapt,"ggd.class":adapt})
        proof["adapter_source_sha256"]=hashlib.sha256((TOOLS/"GeneratedItemCpuAdapter.java").read_bytes()).hexdigest()
        target=root/"crates/mcedit2-iced/tests/fixtures/generated-item-1.21.1.json"
        target.write_bytes(args.output.read_bytes())
        target.with_suffix(".proof.json").write_text(json.dumps(proof,indent=2)+"\n",encoding="utf-8")

if __name__=="__main__":main()
