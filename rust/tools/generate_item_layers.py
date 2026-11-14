"""用准确 1.20.1 的公开模型层接口导出物品专用层；无反射、无图形窗口。"""
import argparse, hashlib, json, os, shutil, subprocess, tempfile, zipfile
from pathlib import Path
LAYERS={"banner#main","bed_head#main","bed_foot#main","chest#main","shulker#main","shield#main","conduit#shell","decorated_pot_base#main","decorated_pot_sides#main","skeleton_skull#main","wither_skeleton_skull#main","zombie_head#main","creeper_head#main","dragon_skull#main","piglin_head#main","player_head#main"}
def main():
 p=argparse.ArgumentParser(description=__doc__)
 for key in ['client','version-json','libraries','output']:p.add_argument('--'+key,type=Path,required=True)
 args=p.parse_args();meta=json.loads(args.version_json.read_text(encoding='utf-8'))
 sha=hashlib.sha1(args.client.read_bytes()).hexdigest()
 if meta['id']!='1.20.1' or sha!='0c3ec587af28e5a785c0b4a7b8a30f9a8f78f838':raise ValueError('接口名称固定为官方 1.20.1')
 source=Path(__file__).with_name('EntityLayerReference.java').read_text(encoding='utf-8')
 for before,after in [('EntityLayerReference','ItemLayerReference'),('ab.a(); akt.a();','aa.a(); acs.a();'),('fyh','feb'),('fyk','fee'),('fbi','eij'),('fbm','ein'),('1.21.1','1.20.1'),('cube.a(new eij().c(),sink,0,0,-1)','cube.a(new eij().c(),sink,0,0,1,1,1,1)'),('cube.a(pose,world,0,0,-1)','cube.a(pose,world,0,0,1,1,1,1)'),('cube.a(pose,sink,0,0,-1)','cube.a(pose,sink,0,0,1,1,1,1)')]:source=source.replace(before,after)
 source=source.replace('a(float x,float y,float z) { vertices.add(new float[]{x,y,z,0,0})','a(double x,double y,double z) { vertices.add(new float[]{(float)x,(float)y,(float)z,0,0})')
 source=source.replace('public ein b(float x,float y,float z)','public ein a(float x,float y,float z)')
 source=source.replace('final List<float[]> vertices', 'public void e() {} public void b(int r,int g,int b,int a) {} public void k() {}\n        final List<float[]> vertices')
 keys=','.join(json.dumps('minecraft:'+key) for key in sorted(LAYERS))
 source=source.replace('var root=entry.getValue().a();','String wanted=entry.getKey().a().toString()+"#"+entry.getKey().b(); if(!Set.of('+keys+').contains(wanted)) continue; var root=entry.getValue().a();')
 java,javac=shutil.which('java'),shutil.which('javac')
 if not java or not javac:raise RuntimeError('需要已有 JDK')
 args.output.parent.mkdir(parents=True,exist_ok=True)
 with tempfile.TemporaryDirectory(prefix='item-layers-',dir=args.output.parent) as temporary:
  work=Path(temporary).resolve();analysis=work/'client.jar'
  with zipfile.ZipFile(args.client)as original,zipfile.ZipFile(analysis,'w')as target:
   for entry in original.infolist():
    if entry.filename.upper().startswith('META-INF/') and entry.filename.upper().endswith(('.SF','.RSA','.DSA','.EC')):continue
    target.writestr(entry,original.read(entry))
  with zipfile.ZipFile(args.client)as original,zipfile.ZipFile(analysis)as target:
   for name in target.namelist():
    if original.read(name)!=target.read(name):raise ValueError('客户端成员发生改变')
  dependencies=[str(analysis)]
  for lib in meta['libraries']:
   artifact=lib.get('downloads',{}).get('artifact',{});path=args.libraries/artifact.get('path','_missing')
   if path.is_file():dependencies.append(str(path.resolve()))
  code=work/'ItemLayerReference.java';code.write_text(source,encoding='utf-8');cp=os.pathsep.join(dependencies)
  subprocess.run([javac,'-proc:none','-encoding','UTF-8','-cp',cp,'-d',str(work),str(code)],check=True,cwd=work)
  raw=work/'raw.json';subprocess.run([java,'-cp',str(work)+os.pathsep+cp,'ItemLayerReference',str(raw)],check=True,cwd=work)
  data=json.loads(raw.read_text(encoding='utf-8'));catalog={'version':data['version'],'layers':data['layers']}
  baseline_path=Path(__file__).resolve().parents[1]/'crates/mcedit2-world/assets/entity-layers-1.21.1.json'
  baseline=json.loads(baseline_path.read_text(encoding='utf-8'))['layers']
  if any(value!=baseline.get(key) for key,value in data['layers'].items()):raise ValueError('真实 1.20.1 模型层与共享层存在差异，必须分别移植')
  proof={'client_sha1':sha,'baseline_catalog_sha256':hashlib.sha256(baseline_path.read_bytes()).hexdigest(),'layers':list(data['layers']),'all_selected_layers_identical':True,'source':'EntityLayerReference.java public ModelPart.visit / Cube.compile'}
  args.output.write_text(json.dumps(proof,indent=2)+'\n',encoding='utf-8',newline='\n')
if __name__=='__main__':main()
