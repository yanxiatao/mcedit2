"""生成专用物品整组顶点参照；复用已校验的官方客户端工具，临时副本结束即清理。"""
import argparse, pathlib, tempfile
from client_reference import run_reference

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for key in ['client-jar','version-json','libraries','output']:p.add_argument('--'+key,type=pathlib.Path,required=True)
 args=p.parse_args();base=pathlib.Path(__file__).resolve().parents[1]/'target/item-reference'
 with tempfile.TemporaryDirectory(prefix='geometry-',dir=base) as temporary:
  run_reference(args,'ItemSpecialReference',pathlib.Path(temporary))
if __name__=='__main__':main()
