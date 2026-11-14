"""使用本机准确客户端及依赖生成资源读取 / 动画元数据参照，不启动游戏。"""
import argparse
import hashlib
import json
import pathlib
import shutil
from client_reference import run_reference


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client-jar', type=pathlib.Path, required=True)
    parser.add_argument('--version-json', type=pathlib.Path, required=True)
    parser.add_argument('--gradle-libraries', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    work = root / 'target/java-resource-reference'
    args.libraries = work / 'libraries'
    manifest = json.loads(args.version_json.read_text(encoding='utf8'))
    names = ('com.google.code.gson:gson:', 'com.google.guava:guava:', 'org.apache.commons:commons-lang3:')
    libraries=[]
    for prefix in names:
        entry = next(item for item in manifest['libraries'] if item['name'].startswith(prefix))
        group, artifact, version = entry['name'].split(':')
        candidates = list((args.gradle_libraries / group / artifact / version).glob('*/*.jar'))
        if len(candidates) != 1:
            raise ValueError(f'需要唯一的准确依赖：{entry["name"]}，找到 {candidates}')
        data=candidates[0].read_bytes()
        if hashlib.sha1(data).hexdigest()!=entry['downloads']['artifact']['sha1']:
            raise ValueError(f'依赖内容与客户端版本声明不同：{entry["name"]}')
        libraries.append({'name':entry['name'],'sha256':hashlib.sha256(data).hexdigest()})
        target = args.libraries / entry['downloads']['artifact']['path']
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(candidates[0], target)
    proof = run_reference(args, 'JavaResourceReference', work)
    proof['libraries']=libraries
    args.output.with_suffix('.proof.json').write_text(json.dumps(proof, ensure_ascii=False, indent=2)+'\n', encoding='utf8', newline='\n')


if __name__ == '__main__':
    main()
