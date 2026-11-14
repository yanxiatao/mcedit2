"""仅将准确旧客户端的 GPU 提交转为完整 CPU 顶点消费；保留其余字节码。"""
import struct

def redirect_draw(data):
    """bul.a(BufferBuilder) -> LegacyEntityReference.capture(BufferBuilder)。无反射。"""
    u2=lambda at:struct.unpack_from('>H',data,at)[0]
    count=u2(8);at=10;cp=[None]*count;i=1
    while i<count:
        tag=data[at];at+=1
        if tag==1:
            length=u2(at);at+=2;cp[i]=data[at:at+length].decode('utf-8');at+=length
        elif tag in (3,4):at+=4
        elif tag in (5,6):at+=8;i+=1
        elif tag in (7,8,16,19,20):at+=2
        elif tag in (9,10,11,12,17,18):at+=4
        elif tag==15:at+=3
        else:raise ValueError('未知常量池类型')
        i+=1
    cp_end=at
    def utf(text):
        raw=text.encode('utf-8');return b'\x01'+struct.pack('>H',len(raw))+raw
    extra=utf('LegacyEntityReference')+utf('capture')+utf('(Lbuk;)V')
    extra+=b'\x07'+struct.pack('>H',count)
    extra+=b'\x0c'+struct.pack('>HH',count+1,count+2)
    extra+=b'\x0a'+struct.pack('>HH',count+3,count+4)
    # header / interfaces / fields 只读跳过，原字节保持不变。
    at+=6;at+=2+2*u2(at)
    fields=u2(at);at+=2
    for _ in range(fields):
        attributes=u2(at+6);at+=8
        for _ in range(attributes):at+=6+struct.unpack_from('>I',data,at+2)[0]
    methods=u2(at);at+=2;start=at;output=[];patched=0
    for _ in range(methods):
        access,name,descriptor,attributes=struct.unpack_from('>HHHH',data,at);at+=8
        output.append(struct.pack('>HHHH',access,name,descriptor,attributes))
        for _ in range(attributes):
            attribute=u2(at);length=struct.unpack_from('>I',data,at+2)[0];raw=data[at:at+6+length];at+=6+length
            if cp[name]=='a'and cp[descriptor]=='(Lbuk;)V'and cp[attribute]=='Code':
                code=b'\x2b\xb8'+struct.pack('>H',count+5)+b'\xb1'
                payload=struct.pack('>HHI',1,2,len(code))+code+struct.pack('>HH',0,0)
                raw=struct.pack('>HI',attribute,len(payload))+payload;patched+=1
            output.append(raw)
    if patched!=1:raise ValueError('未精确定位唯一 GPU 提交方法')
    return data[:8]+struct.pack('>H',count+6)+data[10:cp_end]+extra+data[cp_end:start]+b''.join(output)+data[at:]
