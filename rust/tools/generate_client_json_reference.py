"""用本机准确 Gson 2.10.1 的公开解析接口生成客户端 JSON 语法参照。"""
import argparse, hashlib, json, pathlib, shutil, subprocess, zipfile

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gson-jar',type=pathlib.Path,required=True)
    parser.add_argument('--output',type=pathlib.Path,required=True)
    args=parser.parse_args()
    root=pathlib.Path(__file__).resolve().parents[1]
    jar=args.gson_jar.resolve(strict=True)
    with zipfile.ZipFile(jar) as archive:
        properties=archive.read('META-INF/maven/com.google.code.gson/gson/pom.properties').decode('utf-8')
    metadata=dict(line.strip().split('=',1) for line in properties.splitlines() if line.strip() and not line.lstrip().startswith('#') and '=' in line)
    if metadata.get('version')!='2.10.1': raise ValueError('参照固定使用 Gson 2.10.1')
    java,javac=shutil.which('java'),shutil.which('javac')
    if not java or not javac: raise RuntimeError('需要已有 JDK，不自动安装')
    work=root/'target'/'client-json-reference';work.mkdir(parents=True,exist_ok=True)
    output=args.output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
    source=root/'tools'/'ClientJsonReference.java'
    subprocess.run([javac,'-proc:none','-encoding','UTF-8','-cp',str(jar),'-d',str(work),str(source)],check=True)
    import os
    subprocess.run([java,'-cp',str(work)+os.pathsep+str(jar),'ClientJsonReference',str(output)],check=True)
    proof={'gson_version':'2.10.1','gson_sha256':hashlib.sha256(jar.read_bytes()).hexdigest(),'source_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'output_sha256':hashlib.sha256(output.read_bytes()).hexdigest(),'cases':len(json.loads(output.read_text(encoding='utf-8')))}
    output.with_suffix('.proof.json').write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(proof,ensure_ascii=False))
if __name__=='__main__': main()
