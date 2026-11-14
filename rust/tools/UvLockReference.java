import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.joml.Matrix4f;
import org.joml.Vector4f;

/** 只读调用本机 1.21.1 客户端的 FaceBakery.recomputeUVs，生成独立数值参照。
 * 混淆名依据 Mojang 官方映射；不使用反射，不启动游戏或读取世界。
 */
public final class UvLockReference {
    /** 不同原点、镜像、缩放、剪切和非直角旋转的原始 3×4 行主序根矩阵。 */
    private static final float[][] ROOTS = {
        {1,0,0,0, 0,1,0,0, 0,0,1,0},
        {0,0,1,0, 0,1,0,0, -1,0,0,0},
        {0,-1,0,1, 1,0,0,0, 0,0,1,0},
        {2,0,0,-0.5f, 0,0.5f,0,0.25f, 0,0,1,0},
        {-1,0,0,1, 0,1,0,0, 0,0,1,0},
        {1,0,0,0.125f, 0,1,0,-0.25f, 0,0,1,0.375f},
        {1,0.25f,0,0, 0,1,0.125f,0, 0,0,1,0},
        {0.8480481f,0,0.5299193f,-0.1889837f, 0,1,0,0, -0.5299193f,0,0.8480481f,0.3409356f},
        {0,-2,0,1.5f, 0.5f,0,0,0.25f, 0,0,-1,1},
        {1,0,0,0, 0,0,-1,1, 0,1,0,0},
    };

    /** 显式写数值数组，避免序列化器反射；行内容由客户端实际返回。 */
    private static void numbers(StringBuilder out, float[] values) {
        out.append('[');
        for (int i=0;i<values.length;i++) {
            if(i>0) out.append(',');
            if(!Float.isFinite(values[i])) throw new IllegalArgumentException("非有限客户端结果");
            out.append(Float.toString(values[i]));
        }
        out.append(']');
    }

    /** 参数为明确输出文件；classpath 必须为官方客户端及其实际依赖。 */
    public static void main(String[] args) throws IOException {
        if(args.length!=1) throw new IllegalArgumentException("需要一个输出文件路径");
        StringBuilder out=new StringBuilder("{\"roots\":[");
        for(int i=0;i<ROOTS.length;i++) {
            if(i>0) out.append(',');
            numbers(out,ROOTS[i]);
        }
        out.append("],\"variants\":[");
        for(int x=0;x<360;x+=90) for(int y=0;y<360;y+=90) {
            if(x!=0 || y!=0) out.append(',');
            Matrix4f m=gsn.a(x,y).b().c();
            float[] rows=new float[12];
            for(int row=0;row<3;row++) for(int col=0;col<4;col++) rows[row*4+col]=m.get(col,row);
            numbers(out,rows);
        }
        out.append("],\"cases\":[\n");
        boolean first=true;
        for(int root=0;root<ROOTS.length;root++) {
            float[] r=ROOTS[root];
            Matrix4f raw=new Matrix4f();
            for(int row=0;row<3;row++) raw.setRow(row,new Vector4f(r[row*4],r[row*4+1],r[row*4+2],r[row*4+3]));
            Matrix4f centered=new Matrix4f().translation(-0.5f,-0.5f,-0.5f).mul(raw).translate(0.5f,0.5f,0.5f);
            for(int x=0;x<360;x+=90) for(int y=0;y<360;y+=90) {
                // 直接使用客户端 BlockModelRotation，不能按习惯猜测 X/Y 旋转方向。
                Matrix4f total=new Matrix4f(gsn.a(x,y).b().c()).mul(centered);
                for(ji face:ji.values()) for(int rotation=0;rotation<360;rotation+=90) {
                    float[] uv=rotation==0?new float[]{0,0,16,16}:rotation==90?new float[]{2,3,11,14}:rotation==180?new float[]{16,-4,-8,20}:new float[]{5,15,13,1};
                    gga result=ggd.a(new gga(uv,rotation),face,new j(total));
                    if(!first) out.append(",\n"); first=false;
                    out.append('[').append(root).append(',').append(x).append(',').append(y).append(',').append(face.ordinal()).append(',').append(rotation).append(',');
                    numbers(out,uv); out.append(','); numbers(out,result.a); out.append(',').append(result.b).append(']');
                }
            }
        }
        out.append("\n]}\n");
        Files.writeString(Path.of(args[0]),out,StandardCharsets.UTF_8);
        System.out.println("客户端 UV 数值参照已写入 "+args[0]);
    }
}
