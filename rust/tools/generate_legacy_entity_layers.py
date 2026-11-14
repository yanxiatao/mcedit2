"""导出 1.12.2 原生生物模型；CPU 绘制适配与未改动的模型字节码分别记录。"""
import argparse,hashlib,json,pathlib,tempfile
from client_reference import run_source_reference
from legacy_draw_adapter import redirect_draw

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ['client-jar','version-json','libraries','output']:parser.add_argument('--'+key,type=pathlib.Path,required=True)
    parser.add_argument('--matrix-library',type=pathlib.Path,required=True)
    args=parser.parse_args();output=args.output.resolve()
    source=pathlib.Path(__file__).with_name('LegacyEntityReference.java').read_text(encoding='utf-8')
    base=pathlib.Path(__file__).resolve().parents[1]/'target'
    with tempfile.TemporaryDirectory(prefix='legacy-entity-',dir=base)as directory:
        matrix=args.matrix_library.read_bytes()
        library=next(lib['downloads']['artifact'] for lib in json.loads(args.version_json.read_text(encoding='utf-8'))['libraries'] if lib['name']=='org.lwjgl.lwjgl:lwjgl_util:2.9.4-nightly-20150209')
        if hashlib.sha1(matrix).hexdigest()!=library['sha1']:raise ValueError('矩阵库不是准确客户端声明版本')
        proof=run_source_reference(args,'LegacyEntityReference',source,pathlib.Path(directory),'1.12.2',{'bul.class':redirect_draw},[args.matrix_library])
        proof['matrix_library_sha256']=hashlib.sha256(matrix).hexdigest()
    data=json.loads(output.read_text(encoding='utf-8'))
    output.write_text(json.dumps({'version':data['version'],'layers':data['layers'],'legacy':data['legacy']},separators=(',',':'))+'\n',encoding='utf-8',newline='\n')
    proof['output_sha256']=hashlib.sha256(output.read_bytes()).hexdigest()
    reference=output.parent.parent/'tests/fixtures/entity-layer-reference-1.12.2.json'
    reference.write_text(json.dumps({'version':data['version'],'reference':data['reference'],'poses':[],'living':[]},separators=(',',':'))+'\n',encoding='utf-8',newline='\n')
    proof['reference_sha256']=hashlib.sha256(reference.read_bytes()).hexdigest()
    proof['metadata']=data['metadata']
    proof['adapter_scope']='bul.a(BufferBuilder) GPU 提交改为读取完整四角及 UV 并重置缓冲；ModelBox、TexturedQuad、所有模型和构造器未改动'
    output.with_suffix('.proof.json').write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n')
if __name__=='__main__':main()
