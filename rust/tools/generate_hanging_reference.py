"""从准确客户端实际构造展示框，导出方向 NBT、悬挂坐标、包围盒和内容朝向。无反射或窗口。"""
import argparse,json,pathlib,tempfile
from client_reference import run_source_reference

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for key in ['client-jar','version-json','libraries']:p.add_argument('--'+key,type=pathlib.Path,required=True)
 args=p.parse_args();version=json.loads(args.version_json.read_text(encoding='utf-8'))['id']
 root=pathlib.Path(__file__).resolve().parents[1];base=root/'target/hanging-reference';base.mkdir(parents=True,exist_ok=True)
 if version=='1.21.1':
  bootstrap='ab.a();akt.a();';ctor='new cja((dcw)null,new jd(-19,72,-17),ji.a(facing))';rotate='';tag='ub';save='frame.b(tag);';xyz='frame.dt(),frame.dv(),frame.dz()';angles='frame.dE(),frame.dG()';box='frame.cK()';last='rotation';count=6
 elif version=='1.20.1':
  bootstrap='aa.a();acs.a();';ctor='new bva((cmm)null,new gu(-19,72,-17),ha.a(facing))';rotate='';tag='qr';save='frame.b(tag);';xyz='frame.dn(),frame.dp(),frame.dt()';angles='frame.dy(),frame.dA()';box='frame.cE()';last='rotation';count=6
 elif version=='1.12.2':
  bootstrap='ni.c();';ctor='new acb((amu)null,new et(-19,72,-17),fa.b(facing))';rotate='';tag='fy';save='frame.b(tag);';xyz='frame.p,frame.q,frame.r';angles='frame.v,frame.w';box='frame.bw()';last='rotation';count=4
 else:raise ValueError('未核对该版公开 API')
 source='''import java.util.*;import java.nio.file.*;import com.google.gson.*;
public class HangingReference { public static void main(String[] args)throws Exception {
%s
var result=new ArrayList<Object>();for(int facing=0;facing<%s;facing++){var frame=%s;
for(int rotation=0;rotation<8;rotation++){%s var tag=new %s();%s var box=%s;
result.add(Map.of("facing",(int)tag.f("Facing"),"rotation",%s,"center",new double[]{%s},"angles",new float[]{%s},"bounds",new double[]{box.a,box.b,box.c,box.d,box.e,box.f}));}}
Files.writeString(Path.of(args[0]),new Gson().toJson(result));}}
'''%(bootstrap,count,ctor,rotate,tag,save,box,last,xyz,angles)
 with tempfile.TemporaryDirectory(prefix='cpu-',dir=base) as temporary:
  work=pathlib.Path(temporary);args.output=work/'reference.json'
  proof=run_source_reference(args,'HangingReference',source,work,version)
  output=root/'crates/mcedit2-world/tests/fixtures'/('hanging-reference-'+version+'.json')
  output.write_bytes(args.output.read_bytes())
  output.with_suffix('.proof.json').write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8')

if __name__=='__main__':main()
