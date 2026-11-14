"""从准确客户端的公开旗帜注册表 / 旧枚举 / 数据定义导出图案资源合同。无反射或窗口。"""
import argparse, hashlib, json, pathlib, tempfile, zipfile
from client_reference import run_source_reference
from generate_frame_model_reference import CLIENTS

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for key in ['client-jar','version-json','libraries']:p.add_argument('--'+key,type=pathlib.Path,required=True)
 p.add_argument('--extra-classpath',type=pathlib.Path,action='append',default=[])
 args=p.parse_args();version=json.loads(args.version_json.read_text(encoding='utf-8'))['id']
 if hashlib.sha1(args.client_jar.read_bytes()).hexdigest()!=CLIENTS[version][1]:raise ValueError('客户端 SHA1 与指定版本不符')
 root=pathlib.Path(__file__).resolve().parents[1];target=root/'crates/mcedit2-world/assets'/('banner-patterns-'+version+'.json')
 if version=='1.21.1':
  with zipfile.ZipFile(args.client_jar) as source:
   members=sorted(n for n in source.namelist() if n.startswith('data/minecraft/banner_pattern/') and n.endswith('.json'))
   values={'minecraft:'+n.removeprefix('data/minecraft/banner_pattern/').removesuffix('.json'):json.loads(source.read(n)) for n in members}
  target.write_text(json.dumps(values,indent=2)+'\n',encoding='utf-8')
  proof={'client_sha256':hashlib.sha256(args.client_jar.read_bytes()).hexdigest(),'members':members,'output_sha256':hashlib.sha256(target.read_bytes()).hexdigest()}
 else:
  if version=='1.12.2':
   body='ni.c();for(var value:avg.values()){String asset="minecraft:"+value.a();values.put(asset,Map.of("asset_id",asset,"legacy_hash",value.b(),"banner_texture","minecraft:entity/banner/"+value.a(),"shield_texture","minecraft:entity/shield/"+value.a()));}'
  elif version=='1.20.1':
   body='aa.a();acs.a();for(var entry:jb.al.g()){String banner=cze.a(entry.getKey(),true).toString(),shield=cze.a(entry.getKey(),false).toString();String asset=banner.replace(":entity/banner/",":");if(!shield.replace(":entity/shield/",":").equals(asset))throw new IllegalStateException("旗帜和盾牌资源目录合同改变");values.put(entry.getKey().a().toString(),Map.of("asset_id",asset,"legacy_hash",entry.getValue().a(),"banner_texture",banner,"shield_texture",shield));}'
  else:raise ValueError('未核对的客户端接口')
  code='import java.nio.file.*;import java.util.*;import com.google.gson.*;public class BannerPatternReference {public static void main(String[] args)throws Exception {var values=new TreeMap<String,Object>();'+body+'Files.writeString(Path.of(args[0]),new Gson().toJson(values));}}'
  with tempfile.TemporaryDirectory(prefix='banner-pattern-reference-',dir=root/'target') as temporary:
   work=pathlib.Path(temporary);args.output=work/'patterns.json'
   proof=run_source_reference(args,'BannerPatternReference',code,work,version,extra_classpath=args.extra_classpath)
   proof['source_sha256']=hashlib.sha256(code.encode()).hexdigest()
   proof['extra_classpath']=[{'name':p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in args.extra_classpath]
   target.write_bytes(args.output.read_bytes())
 target.with_suffix('.proof.json').write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8')

if __name__=='__main__':main()
