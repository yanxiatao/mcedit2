"""为原生窗口集成检查创建独立的 1.21.1 小世界；不修改任何已有存档或客户端。"""
import argparse
import gzip
import json
import os
from pathlib import Path
import struct
import uuid
import zlib
import zipfile


def text(value):
    raw=value.encode("utf-8")
    return struct.pack(">H",len(raw))+raw


def payload(kind,value):
    if kind in [1,2,3,4,5,6]:return struct.pack({1:">b",2:">h",3:">i",4:">q",5:">f",6:">d"}[kind],value)
    if kind==7:return struct.pack(">i",len(value))+bytes(value)
    if kind==8:return text(value)
    if kind==9:
        child,items=value
        return bytes([child])+struct.pack(">i",len(items))+b"".join(payload(child,v) for v in items)
    if kind==10:return b"".join(bytes([t])+text(name)+payload(t,v) for name,(t,v) in value.items())+b"\0"
    if kind in [11,12]:return struct.pack(">i",len(value))+b"".join(struct.pack(">i" if kind==11 else ">q",v) for v in value)
    raise ValueError(f"未定义的 NBT 类型 {kind}")


def nbt(value):return b"\x0a\0\0"+payload(10,value)
def compound(**fields):return 10,fields
def items(values):return 9,(10,values)
def strings(values):return 9,(8,values)
def coordinates(values):return 9,(6,values)
def rotation(values):return 9,(5,values)


def write_region(path,chunks):
    result=bytearray(8192)
    for (cx,cz),raw in chunks:
        data=zlib.compress(raw);count=(len(data)+5+4095)//4096;start=len(result)//4096
        offset=((cx%32)+(cz%32)*32)*4
        result[offset:offset+4]=struct.pack(">I",start<<8|count)
        result.extend(struct.pack(">I",len(data)+1)+b"\2"+data+bytes(count*4096-len(data)-5))
    path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(result)


def entity(index,id,pos,**fields):
    return dict(id=(8,id),Pos=coordinates(pos),Rotation=rotation([0.,0.]),UUID=(11,[0,42,0,index]),**fields)


def png(path):
    def chunk(name,data):return struct.pack(">I",len(data))+name+data+struct.pack(">I",zlib.crc32(name+data))
    raw=b"".join(b"\0"+bytes(c for x in range(32) for c in ([220,80,40,255] if (x//4+y//4)%2 else [30,140,220,255])) for y in range(32))
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(b"\x89PNG\r\n\x1a\n"+chunk(b"IHDR",struct.pack(">IIBBBBB",32,32,8,6,0,0,0))+chunk(b"IDAT",zlib.compress(raw))+chunk(b"IEND",b""))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output",type=Path);parser.add_argument("--client",type=Path,required=True)
    parser.add_argument("--launcher-root",type=Path,required=True)
    parser.add_argument("--rebuild",action="store_true",help="重建已有的本工具夹具，保留同一配置目录")
    args=parser.parse_args();root=args.output.resolve()
    if root.exists():
        manifest=json.loads((root/"manifest.json").read_text(encoding="utf-8"))
        if not args.rebuild or Path(manifest["world"]).resolve()!=root/".minecraft/saves/通用渲染集成":
            raise ValueError("输出目录不是明确选择重建的同一份窗口夹具")
    else:root.mkdir(parents=True)
    game=root/".minecraft";world=game/"saves/通用渲染集成"
    world.mkdir(parents=True,exist_ok=True)
    client=game/"versions/1.21.1/1.21.1.jar";client.parent.mkdir(parents=True,exist_ok=True)
    with zipfile.ZipFile(args.client) as archive:
        if json.loads(archive.read("version.json"))["id"]!="1.21.1":raise ValueError("需要准确的 1.21.1 客户端")
    if not client.exists():os.link(args.client.resolve(strict=True),client)
    elif not os.path.samefile(client,args.client):raise ValueError("既有客户端链接与指定来源不同")
    metadata_path=args.launcher_root/"meta/net.minecraft/1.21.1.json"
    metadata=json.loads(metadata_path.read_text(encoding="utf-8"));index_id=metadata["assetIndex"]["id"]
    (client.parent/"1.21.1.json").write_text(json.dumps({"id":"1.21.1","assetIndex":metadata["assetIndex"]}),encoding="utf-8")
    original_index=args.launcher_root/"assets/indexes"/(index_id+".json")
    index=game/"assets/indexes"/(index_id+".json");index.parent.mkdir(parents=True,exist_ok=True)
    if not index.exists():os.link(original_index,index)
    elif not os.path.samefile(index,original_index):raise ValueError("既有资产索引与指定来源不同")
    asset_count=0
    for name,entry in json.loads(original_index.read_text(encoding="utf-8"))["objects"].items():
        parts=name.split("/",2)
        if len(parts)!=3 or parts[1] not in {"font","lang","textures","models","blockstates","atlases"}:continue
        hash=entry["hash"];source=args.launcher_root/"assets/objects"/hash[:2]/hash;target=game/"assets/objects"/hash[:2]/hash
        if source.stat().st_size!=entry["size"]:raise ValueError("声明的资产大小不同："+name)
        target.parent.mkdir(parents=True,exist_ok=True)
        if not target.exists():os.link(source,target)
        elif not os.path.samefile(source,target):raise ValueError("既有资产链接与指定来源不同："+name)
        asset_count+=1
    player=entity(100,"minecraft:player",[8.,67.,-10.],Dimension=(8,"minecraft:overworld"),Health=(5,20.),Air=(2,300),foodLevel=(3,20),XpLevel=(3,10),playerGameType=(3,1),SelectedItemSlot=(3,0),
        abilities=compound(flying=(1,0),mayfly=(1,1),flySpeed=(5,0.05)),Inventory=items([dict(Slot=(1,0),id=(8,"minecraft:stone"),count=(3,64))]),EnderItems=items([]))
    level=dict(Data=compound(DataVersion=(3,3955),version=(3,19133),Version=compound(Id=(3,3955),Name=(8,"1.21.1")),LevelName=(8,"通用渲染集成"),RandomSeed=(4,42),Time=(4,48000),DayTime=(4,6000),Difficulty=(1,2),GameType=(3,1),hardcore=(1,0),allowCommands=(1,1),SpawnX=(3,8),SpawnY=(3,67),SpawnZ=(3,0),Player=(10,player),GameRules=compound(doDaylightCycle=(8,"true")),WorldGenSettings=compound(dimensions=compound(**{"minecraft:overworld":compound(type=(8,"minecraft:overworld"))})),DataPacks=compound(Enabled=strings(["vanilla","file/editor-ui"]),Disabled=strings([]))))
    (world/"level.dat").write_bytes(gzip.compress(nbt(level)))
    player_id=uuid.UUID(bytes=struct.pack(">iiii",0,42,0,100))
    (world/"playerdata").mkdir(exist_ok=True);(world/f"playerdata/{player_id}.dat").write_bytes(gzip.compress(nbt(dict(DataVersion=(3,3955),**player))))
    pack=world/"datapacks/editor-ui";definition=pack/"data/generic/painting_variant/poster.json";definition.parent.mkdir(parents=True,exist_ok=True)
    (pack/"pack.mcmeta").write_text(json.dumps({"pack":{"pack_format":48,"description":"窗口集成数据"}},ensure_ascii=False),encoding="utf-8")
    definition.write_text(json.dumps({"width":3,"height":2,"asset_id":"generic:poster"}),encoding="utf-8")
    pattern=pack/"data/generic/banner_pattern/emblem.json";pattern.parent.mkdir(parents=True,exist_ok=True)
    pattern.write_text(json.dumps({"asset_id":"generic:checker","translation_key":"pattern.generic.emblem"}),encoding="utf-8")
    assets=game/"resourcepacks/editor-ui";png(assets/"assets/generic/textures/painting/poster.png")
    for family in ["banner","shield"]:png(assets/f"assets/generic/textures/entity/{family}/checker.png")
    (assets/"pack.mcmeta").write_text(json.dumps({"pack":{"pack_format":34,"description":"窗口集成资源"}},ensure_ascii=False),encoding="utf-8")
    (game/"options.txt").write_text('resourcePacks:["vanilla","file/editor-ui"]\n',encoding="utf-8")
    palette=[dict(Name=(8,name)) for name in ["minecraft:air","minecraft:stone","minecraft:glass","minecraft:oak_planks"]]
    palette.extend([dict(Name=(8,"minecraft:chest"),Properties=compound(facing=(8,"north"),type=(8,"single"),waterlogged=(8,"false"))),dict(Name=(8,"minecraft:oak_wall_sign"),Properties=compound(facing=(8,"north"),waterlogged=(8,"false")))])
    palette.append(dict(Name=(8,"minecraft:white_wall_banner"),Properties=compound(facing=(8,"north"))))
    regions={}
    for cz in range(-1,2):
        for cx in range(-1,2):
            sections=[]
            for sy in range(-4,20):
                cells=[0]*4096
                if sy==3:cells[15*256:]=[1]*256
                if (cx,cz)==(0,0) and sy==4:
                    for y in range(7):
                        for x in range(2,15):cells[y*256+8*16+x]=3
                    cells[4*16+3]=4;cells[2*256+7*16+2]=5;cells[3*256+7*16+6]=6
                data=[sum(cells[i+j]<<(j*4) for j in range(16)) for i in range(0,4096,16)]
                states=compound(palette=items(palette),data=(12,data)) if any(cells) else compound(palette=items([palette[0]]))
                sections.append(dict(Y=(1,sy),block_states=states,biomes=compound(palette=strings(["minecraft:plains"])),SkyLight=(7,bytes([255])*2048),BlockLight=(7,bytes(2048))))
            block_entities=[]
            if (cx,cz)==(0,0):
                side=dict(messages=strings([json.dumps(s,ensure_ascii=False) for s in ["通用适配","窗口集成检查","告示牌",""]]),color=(8,"black"),has_glowing_text=(1,0))
                block_entities=[dict(id=(8,"minecraft:chest"),x=(3,3),y=(3,64),z=(3,4),Items=items([dict(Slot=(1,0),id=(8,"minecraft:diamond_sword"),count=(3,3))])),dict(id=(8,"minecraft:sign"),x=(3,2),y=(3,66),z=(3,7),front_text=(10,side),back_text=(10,side),is_waxed=(1,0))]
                block_entities.append(dict(id=(8,"minecraft:banner"),x=(3,6),y=(3,67),z=(3,7),patterns=items([dict(pattern=(8,"generic:emblem"),color=(8,"white"))])))
            raw=nbt(dict(DataVersion=(3,3955),xPos=(3,cx),zPos=(3,cz),yPos=(3,-4),Status=(8,"minecraft:full"),LastUpdate=(4,0),InhabitedTime=(4,100),isLightOn=(1,1),sections=items(sections),Heightmaps=compound(),block_entities=items(block_entities),block_ticks=items([]),fluid_ticks=items([])))
            regions.setdefault((cx//32,cz//32),[]).append(((cx,cz),raw))
    for (rx,rz),chunks in regions.items():write_region(world/f"region/r.{rx}.{rz}.mca",chunks)
    objects=[]
    for i,(x,id) in enumerate([(4,"minecraft:kebab"),(8,"generic:poster")],1):
        h=2 if i==2 else 1
        objects.append(entity(i,"minecraft:painting",[x+0.5,66.5+(0.5 if h%2==0 else 0),7.96875],TileX=(3,x),TileY=(3,66),TileZ=(3,7),facing=(1,2),variant=(8,id)))
    for i,(x,id) in enumerate([(1,"minecraft:filled_map"),(11,"minecraft:diamond_sword"),(13,"minecraft:potion")],10):
        components=compound(**{"minecraft:map_id":(3,7)}) if i==10 else compound(**{"minecraft:potion_contents":compound(potion=(8,"minecraft:healing"))}) if i==12 else compound()
        objects.append(entity(i,"minecraft:item_frame",[x+0.5,66.5,7.96875],TileX=(3,x),TileY=(3,66),TileZ=(3,7),Facing=(1,2),ItemRotation=(1,3 if i==11 else 0),Invisible=(1,1 if i==12 else 0),Item=compound(id=(8,id),count=(3,1),components=components)))
    objects.append(entity(30,"minecraft:pig",[4.,64.,3.],Health=(5,10.),Age=(3,0)))
    for i,(x,id) in enumerate([(3,"minecraft:clock"),(5,"minecraft:compass"),(7,"minecraft:shield"),(11,"minecraft:white_banner")],40):
        components=compound(**{"minecraft:banner_patterns":items([dict(pattern=(8,"generic:emblem"),color=(8,"white"))])}) if i>=42 else compound()
        objects.append(entity(i,"minecraft:item_frame",[x+0.5,69.5,7.96875],TileX=(3,x),TileY=(3,69),TileZ=(3,7),Facing=(1,2),ItemRotation=(1,0),Item=compound(id=(8,id),count=(3,1),components=components)))
    write_region(world/"entities/r.0.0.mca",[((0,0),nbt(dict(DataVersion=(3,3955),Position=(11,[0,0]),Entities=items(objects))))])
    (world/"data").mkdir(exist_ok=True);(world/"data/map_7.dat").write_bytes(gzip.compress(nbt(dict(DataVersion=(3,3955),data=compound(dimension=(8,"minecraft:overworld"),scale=(1,0),xCenter=(3,0),zCenter=(3,0),colors=(7,bytes(18 if i%128<64 else 34 for i in range(128*128))))))))
    (root/"manifest.json").write_text(json.dumps({"world":str(world),"client_hardlink":str(client),"asset_metadata":str(metadata_path),"asset_index":str(index),"linked_assets":asset_count,"scope":"仅原生窗口 / GPU 集成，不作为游戏重载证明"},ensure_ascii=False,indent=2),encoding="utf-8")
    print(world)


if __name__=="__main__":main()
