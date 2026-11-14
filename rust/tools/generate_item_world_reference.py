"""准确原版 CPU 指南针 / 展示框朝向和天体时间整批参照。无反射，不打开窗口。"""
import argparse, hashlib, json, pathlib, subprocess, tempfile
from client_reference import run_source_reference

TOOLS = pathlib.Path(__file__).resolve().parent

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for key in ['client-jar','version-json','libraries']:p.add_argument('--'+key,type=pathlib.Path,required=True)
 p.add_argument('--extra-classpath',type=pathlib.Path,action='append',default=[])
 args=p.parse_args();version=json.loads(args.version_json.read_text(encoding='utf-8'))['id']
 if version=='1.21.1':
  clock='dwy';frame='cja';compass='gpr';boot='ab.a();akt.a();';ctor='new cja((dcw)null,new jd(-19,72,-17),ji.a(facing))';yaw='frame.dF()';count=6;target='new jd(target[0],target[1],target[2])';property='compass.a(frame,1L,pos)';xyz='frame.dt(),frame.dv(),frame.dz()'
 elif version=='1.20.1':
  clock='dfk';frame='bva';compass='fub';boot='aa.a();acs.a();';ctor='new bva((cmm)null,new gu(-19,72,-17),ha.a(facing))';yaw='frame.dz()';count=6;target='new gu(target[0],target[1],target[2])';property='compass.a(frame,1L,pos)';xyz='frame.dn(),frame.dp(),frame.dt()'
 elif version=='1.12.2':
  clock='aym';frame='acb';compass='ahn$1';boot='ni.c();';ctor='new acb((amu)null,new et(-19,72,-17),fa.b(facing))';yaw='compass.a(frame)';count=4;target='new et(target[0],target[1],target[2])';property='rk.b((float)(0.5-(rk.b(compass.a(frame)/360.,1.)-0.25-compass.a((amu)null,frame)/(double)(float)(2*Math.PI))),1f)';xyz='frame.p,frame.q,frame.r'
 else:raise ValueError('没有该版本的准确接口')
 modern=version!='1.12.2'
 hook=('static OptionalLong fixed=OptionalLong.empty(); public static OptionalLong fixedTime('+clock+' ignored){return fixed;}') if modern else 'static et pos; public static et spawn(amu ignored){return pos;}'
 source='''import java.util.*;import java.nio.file.*;import com.google.gson.*;
public class ItemWorldReference {
%s
public static void main(String[] args)throws Exception {
%s
var compass=%s;var frames=new ArrayList<Object>();var times=new ArrayList<Object>();
for(int facing=0;facing<%d;facing++){var frame=%s;
for(int rotation=0;rotation<8;rotation++){frame.a(rotation,false);
for(var target:List.of(new int[]{0,64,0},new int[]{-19,72,-17},new int[]{-80,65,20},new int[]{50,80,-90})){
%s
frames.add(Map.of("facing",facing,"rotation",rotation,"yaw",%s,"position",new double[]{%s},"target",target,"angle",%s));
}}}
%s
Files.writeString(Path.of(args[0]),new Gson().toJson(Map.of("frames",frames,"times",times)));
}}
'''%(hook,boot,('new '+compass+'((level,stack,entity)->null)') if modern else 'new ahn$1(new ahn())',count,ctor,('var pos='+target+';') if modern else ('pos='+target+';'),yaw,xyz,property,
 ('for(long fixedTime:new long[]{-1,6000,18000,28000}){fixed=fixedTime==-1?OptionalLong.empty():OptionalLong.of(fixedTime);for(long day:new long[]{0,1,5999,6000,12000,18000,23999,24000,24001,123456789})times.add(Map.of("day",day,"fixed",fixedTime,"value",'+clock+'.sample(null,day)));}') if modern else 'for(long day:new long[]{0,1,5999,6000,12000,18000,23999,24000,24001,123456789})times.add(Map.of("day",day,"fixed",-1,"value",aym.sample(null,day,1f)));')
 root=TOOLS.parent;base=root/'target/item-world-reference';base.mkdir(exist_ok=True)
 with tempfile.TemporaryDirectory(prefix='cpu-',dir=base) as temporary:
  work=pathlib.Path(temporary)
  subprocess.run(['javac','--add-exports','java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED','-d',str(work),str(TOOLS/'ItemWorldCpuAdapter.java')],check=True)
  def adapt(data):
   src=work/'source.class';dst=work/'adapted.class';src.write_bytes(data)
   subprocess.run(['java','--add-exports','java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED','-cp',str(work),'ItemWorldCpuAdapter',str(src),str(dst)],check=True)
   return dst.read_bytes()
  args.output=work/'reference.json'
  proof=run_source_reference(args,'ItemWorldReference',source,work,version,{c+'.class':adapt for c in [clock,frame,compass]},args.extra_classpath)
  proof['extra_classpath']=[{'name':p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in args.extra_classpath]
  proof['adapter_source_sha256']=hashlib.sha256((TOOLS/'ItemWorldCpuAdapter.java').read_bytes()).hexdigest()
  proof['source_sha256']=hashlib.sha256(source.encode()).hexdigest()
  target=root/'crates/mcedit2-iced/tests/fixtures'/('item-world-'+version+'.json')
  target.write_bytes(args.output.read_bytes());target.with_suffix('.proof.json').write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8')

if __name__=='__main__':main()
