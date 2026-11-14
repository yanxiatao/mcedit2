"""准确客户端的公开模型 renderToBuffer 输出，集中覆盖成年 / 幼年及头身姿态。"""
import argparse,json,pathlib,tempfile
from client_reference import run_source_reference

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for key in ['client-jar','version-json','libraries','output']:p.add_argument('--'+key,type=pathlib.Path,required=True)
    args=p.parse_args();version=json.loads(args.version_json.read_text(encoding='utf-8'))['id']
    if version=='1.21.1':
        entity,root,stack,layers,vertex='fvk','fyk','fbi','fyh','fbm';bootstrap='ab.a();akt.a();';color='-1'
        classes={'cow':'fvd','pig':'fwm','zombie':'fyc','skeleton':'fxf','sheep':'fxa','spider':'fxl'}
        sink='''public fbm a(float x,float y,float z){vertices.add(new float[]{x,y,z,0,0});return this;}
public fbm a(float u,float v){var p=vertices.getLast();p[3]=u;p[4]=v;return this;}
public fbm a(int r,int g,int b,int a){return this;} public fbm a(int u,int v){return this;}
public fbm b(int u,int v){return this;} public fbm b(float x,float y,float z){return this;}'''
    elif version=='1.20.1':
        entity,root,stack,layers,vertex='fbf','fee','eij','feb','ein';bootstrap='aa.a();acs.a();';color='1,1,1,1'
        classes={'cow':'fay','pig':'fch','zombie':'fdw','skeleton':'fda','sheep':'fcv','spider':'fdg'}
        sink='''float[] pending;public ein a(double x,double y,double z){pending=new float[]{(float)x,(float)y,(float)z,0,0};return this;}
public ein a(float u,float v){pending[3]=u;pending[4]=v;return this;}
public void e(){vertices.add(pending);pending=null;} public ein a(int r,int g,int b,int a){return this;}
public ein a(int u,int v){return this;}public ein b(int u,int v){return this;}public ein a(float x,float y,float z){return this;}
public void b(int r,int g,int b,int a){throw new UnsupportedOperationException();} public void k(){throw new UnsupportedOperationException();}'''
    else:raise ValueError('模型参照版本未绑定')
    factories=''.join(f'case "{id}"->new {cls}<>(root);'for id,cls in classes.items())
    source=f'''import java.util.*;import java.nio.file.*;import com.google.gson.Gson;
public class YoungModelReference{{
static class Sink implements {vertex}{{final List<float[]> vertices=new ArrayList<>();{sink}}}
public static void main(String[] args)throws Exception{{{bootstrap}
var cases=new ArrayList<Object>();
for(String id:List.of("cow","pig","zombie","skeleton","sheep","spider"))for(boolean young:id.equals("spider")?List.of(false):List.of(false,true))for(float pitch:List.of(0f,31f)){{
String layer="minecraft:"+id+"#main";
var root={layers}.a().entrySet().stream().filter(e->(e.getKey().a()+"#"+e.getKey().b()).equals(layer)).findFirst().orElseThrow().getValue().a();
{entity}<?> model=switch(id){{{factories}default->throw new IllegalArgumentException(id);}};
if(id.equals("spider"))model.a(null,0,0,0,0,0);
model.e=young;root.b("head").e=pitch*((float)Math.PI/180f);
if(id.equals("zombie")||id.equals("skeleton"))root.b("hat").e=root.b("head").e;
float yaw=37.25f;var pose=new {stack}();pose.a(a.d.rotationDegrees(180-yaw));pose.b(-1,-1,1);pose.a(0,-1.501f,0);
var sink=new Sink();model.a(pose,sink,0,0,{color});
cases.add(Map.of("id",id,"layer",layer,"young",young,"yaw",yaw,"pitch",pitch,"vertices",sink.vertices));
}}
Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("version","{version}","cases",cases)));
System.out.println("头身分组参照："+cases.size());
}}}}'''
    base=pathlib.Path(__file__).resolve().parents[1]/'target'
    with tempfile.TemporaryDirectory(prefix='young-model-',dir=base)as directory:
        proof=run_source_reference(args,'YoungModelReference',source,pathlib.Path(directory),version)
    args.output.with_suffix('.proof.json').write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8',newline='\n')
if __name__=='__main__':main()
