"""从三版准确客户端导出挂画目录、原生数值顶点和包围盒。只在临时分析副本适配 CPU 边界。"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile
import zipfile
from client_reference import run_source_reference

TOOLS = pathlib.Path(__file__).resolve().parent
ROOT = TOOLS.parent

COMMON = '''import java.nio.*; import java.nio.file.*; import java.util.*;
import com.google.gson.*;
public class PaintingReference {
static List<float[]> vertices; static int width,height;
static Map<String,Object> catalog=new TreeMap<>(), geometry=new TreeMap<>();
static Map<String,Object> entry(int w,int h,String asset){return new TreeMap<>(Map.of("width",w,"height",h,"asset_id",asset));}
%s
public static void main(String[] args)throws Exception {
%s
Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("catalog",catalog,"geometry",geometry)));
}}
'''

def source(version, client):
    if version == '1.12.2':
        methods = '''public static void light(cai renderer,acd entity,float x,float y){}
public static void draw(bve draw){var buffer=draw.c();buffer.e();
if(buffer.h()!=24||buffer.g().g()!=24)throw new IllegalStateException("挂画缓冲格式改变");
ByteBuffer raw=buffer.f().duplicate().order(ByteOrder.nativeOrder());
for(int i=0;i<24;i++){int at=i*24;vertices.add(new float[]{raw.getFloat(at)/16,raw.getFloat(at+4)/16,raw.getFloat(at+8)/16,raw.getFloat(at+12),raw.getFloat(at+16)});}buffer.b();}
'''
        main = '''ni.c();for(var art:((acd.a)null).values()){
String id="minecraft:"+art.name().toLowerCase(Locale.ROOT);
var record=entry(art.C/16,art.D/16,id.equals("minecraft:aztec_2")?"minecraft:aztec2":id);record.put("motive",art.B);record.put("legacy_uv",new int[]{art.E,art.F});catalog.put(id,record);
vertices=new ArrayList<>();cai.a((cai)null,(acd)null,art.C,art.D,art.E,art.F);geometry.put(id,Map.of("vertices",vertices));}
'''
    else:
        modern = version == '1.21.1'
        entity, sprite, direction, world, view, pos, pose, sink, renderer = (
            ('cjc','gql','ji','dcw','dbz','jd','fbi','fbm','glu') if modern else
            ('bvc','fuv','ha','cmm','clp','gu','eij','ein','fqi'))
        methods = f'''public static int entityInt({entity} e){{return 0;}}
public static double entityDouble({entity} e){{return 0;}}
public static {direction} direction({entity} e){{return {direction}.b(0);}}
public static {world} world({entity} e){{return null;}}
public static int light({view} w,{pos} p){{return 0;}}
public static float min({sprite} s){{return 0;}} public static float max({sprite} s){{return 1;}}
public static float uv({sprite} s,{'float' if modern else 'double'} v){{return (float)v{' ' if modern else '/16'};}}
static class Sink implements {sink} {{
float[] pending;
public {sink} a({'float' if modern else 'double'} x,{'float' if modern else 'double'} y,{'float' if modern else 'double'} z){{pending=new float[]{{(float)x,(float)y,(float)z,0,0}};{'vertices.add(pending);' if modern else ''}return this;}}
public {sink} a(float u,float v){{pending[3]=u;pending[4]=v;return this;}}
public {sink} a(int r,int g,int b,int a){{return this;}}
public {sink} a(int u,int v){{return this;}} public {sink} b(int u,int v){{return this;}}
public {sink} {'b' if modern else 'a'}(float x,float y,float z){{return this;}}
{'' if modern else 'public void e(){vertices.add(pending);} public void b(int r,int g,int b,int a){throw new UnsupportedOperationException();} public void k(){throw new UnsupportedOperationException();}'}
}}
'''
        if modern:
            methods += '''public static jm<cjd> variant(cjc e){return jm.a(new cjd(width,height,akr.a("minecraft:kebab")));}
'''
            with zipfile.ZipFile(client) as jar:
                definitions = {name.removeprefix('data/minecraft/painting_variant/').removesuffix('.json'): json.loads(jar.read(name)) for name in jar.namelist() if name.startswith('data/minecraft/painting_variant/') and name.endswith('.json')}
            main = 'ab.a();akt.a();\n'
            for name, record in sorted(definitions.items()):
                main += f'catalog.put("minecraft:{name}",entry({record["width"]},{record["height"]},"{record["asset_id"]}"));\n'
        else:
            main = '''aa.a();acs.a();for(var variant:jb.m){String id=jb.m.b(variant).toString();catalog.put(id,entry(variant.a()/16,variant.b()/16,id));}
'''
        main += f'''for(var art:catalog.entrySet()){{var record=(Map<String,Object>)art.getValue();width=(int)record.get("width");height=(int)record.get("height");
var directions=new ArrayList<Object>();for(int facing=0;facing<4;facing++){{vertices=new ArrayList<>();var pose=new {pose}();
pose.a(new org.joml.Quaternionf().rotationY((float)Math.toRadians(180-facing*90)));
{'' if modern else 'pose.b(1f/16,1f/16,1f/16);'}
{renderer}.a(null,pose,new Sink(),null,width*{1 if modern else 16},height*{1 if modern else 16},null,null);
var direction=new TreeMap<String,Object>();direction.put("vertices",vertices);
{ 'var box=cjc.a((cjc)null,new jd(-19,72,-17),ji.b(facing));direction.put("bounds",new double[]{box.a,box.b,box.c,box.d,box.e,box.f});' if modern else ''}
directions.add(direction);}}geometry.put(art.getKey(),directions);}}
'''
    return COMMON % (methods, main)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['client-jar','version-json','libraries']:
        parser.add_argument('--'+key,type=pathlib.Path,required=True)
    args=parser.parse_args()
    version=json.loads(args.version_json.read_text(encoding='utf-8'))['id']
    if version not in ('1.12.2','1.20.1','1.21.1'):raise ValueError('未绑定该版公开接口')
    base=ROOT/'target/painting-reference';base.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='cpu-',dir=base) as directory:
        work=pathlib.Path(directory)
        subprocess.run(['javac','--add-exports','java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED','-d',str(work),str(TOOLS/'PaintingCpuAdapter.java')],check=True)
        def adapt(payload):
            original=work/'original.class';changed=work/'adapted.class';original.write_bytes(payload)
            subprocess.run(['java','--add-exports','java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED','-cp',str(work),'PaintingCpuAdapter',version,str(original),str(changed)],check=True)
            return changed.read_bytes()
        renderer={'1.12.2':'cai','1.20.1':'fqi','1.21.1':'glu'}[version]
        adapters={renderer+'.class':adapt}
        if version=='1.21.1':adapters['cjc.class']=adapt
        args.output=work/'reference.json'
        proof=run_source_reference(args,'PaintingReference',source(version,args.client_jar),work,version,adapters)
        proof['adapter_source_sha256']=hashlib.sha256((TOOLS/'PaintingCpuAdapter.java').read_bytes()).hexdigest()
        result=json.loads(args.output.read_text(encoding='utf-8'))
        for folder,name,value in [('crates/mcedit2-world/assets','paintings-'+version,result['catalog']),('crates/mcedit2-world/tests/fixtures','painting-reference-'+version,result['geometry'])]:
            path=ROOT/folder/(name+'.json');path.write_text(json.dumps(value,separators=(',',':'),ensure_ascii=False)+'\n',encoding='utf-8')
            proof['catalog_sha256' if folder.endswith('assets') else 'geometry_sha256']=hashlib.sha256(path.read_bytes()).hexdigest()
        path=ROOT/'crates/mcedit2-world/assets'/('paintings-'+version+'.proof.json')
        path.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')

if __name__=='__main__':main()
