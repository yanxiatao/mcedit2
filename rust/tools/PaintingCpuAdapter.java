import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
/** 只改分析副本的调用边界：静态数值入口、CPU 光照 / UV 和旧版缓冲消费。无反射。 */
public final class PaintingCpuAdapter implements Opcodes {
 public static void main(String[] args)throws Exception {
  String version=args[0];byte[] input=Files.readAllBytes(Path.of(args[1]));
  String renderer=version.equals("1.21.1")?"glu":version.equals("1.20.1")?"fqi":"cai";
  String entity=version.equals("1.21.1")?"cjc":version.equals("1.20.1")?"bvc":"acd";
  String sprite=version.equals("1.21.1")?"gql":"fuv";
  var reader=new ClassReader(input);var output=new ClassWriter(0);
  reader.accept(new ClassVisitor(ASM8,output){String own;
   @Override public void visit(int ver,int flags,String name,String signature,String parent,String[] interfaces){own=name;super.visit(ver,flags,name,signature,parent,interfaces);}
   @Override public MethodVisitor visitMethod(int flags,String name,String desc,String signature,String[] exceptions){
    boolean geometry=own.equals(renderer)&&name.equals("a")&&((version.equals("1.12.2")&&desc.equals("(Lacd;IIII)V"))||(!version.equals("1.12.2")&&desc.contains("L"+entity+";II")));
    boolean vertex=own.equals(renderer)&&name.equals("a")&&desc.endsWith("FFFFFIIII)V");
    boolean bounds=own.equals("cjc")&&((name.equals("a")&&desc.equals("(Ljd;Lji;)Lewx;"))||(name.equals("b")&&desc.equals("(I)D")));
    if(!geometry&&!vertex&&!bounds)return super.visitMethod(flags,name,desc,signature,exceptions);
    var method=super.visitMethod((flags&~(ACC_PRIVATE|ACC_PROTECTED))|ACC_PUBLIC|ACC_STATIC,name,"(L"+own+";"+desc.substring(1),null,exceptions);
    return new MethodVisitor(ASM8,method){
     @Override public void visitFieldInsn(int opcode,String owner,String field,String d){
      if(owner.equals("gqc")&&field.equals("d")&&d.equals("I")){super.visitInsn(ICONST_0);return;}
      super.visitFieldInsn(opcode,owner,field,d);
     }
     @Override public void visitMethodInsn(int opcode,String owner,String method,String d,boolean itf){
      if(owner.equals(renderer)&&method.equals("a")&&d.endsWith("FFFFFIIII)V")){
       super.visitMethodInsn(INVOKESTATIC,owner,method,"(L"+owner+";"+d.substring(1),false);return;
      }
      if(own.equals("cjc")&&owner.equals("cjc")&&method.equals("b")&&d.equals("(I)D")){
       super.visitMethodInsn(INVOKESTATIC,owner,method,"(Lcjc;I)D",false);return;
      }
      if(owner.equals(entity)){
       String target=switch(d){case "()I"->"entityInt";case "()D"->"entityDouble";case "()Lji;","()Lha;"->"direction";case "()Ldcw;","()Lcmm;"->"world";case "()Ljm;"->"variant";default->null;};
       if(target==null)throw new IllegalStateException("未绑定的实体调用 "+owner+"."+method+d);
       super.visitMethodInsn(INVOKESTATIC,"PaintingReference",target,"(L"+owner+";"+d.substring(1),false);return;
      }
      if(owner.equals(sprite)){
       String target=d.equals("()F")?(method.equals("c")||method.equals("g")?"min":"max"):"uv";
       super.visitMethodInsn(INVOKESTATIC,"PaintingReference",target,"(L"+owner+";"+d.substring(1),false);return;
      }
      if((owner.equals("gex")||owner.equals("fjv"))&&d.endsWith(";)I")){
       super.visitMethodInsn(INVOKESTATIC,"PaintingReference","light",d,false);return;
      }
      if(version.equals("1.12.2")&&owner.equals("cai")&&d.equals("(Lacd;FF)V")){
       super.visitMethodInsn(INVOKESTATIC,"PaintingReference","light","(Lcai;Lacd;FF)V",false);return;
      }
      if(version.equals("1.12.2")&&owner.equals("bve")&&method.equals("b")&&d.equals("()V")){
       super.visitMethodInsn(INVOKESTATIC,"PaintingReference","draw","(Lbve;)V",false);return;
      }
      super.visitMethodInsn(opcode,owner,method,d,itf);
     }
    };
   }
  },0);
  Files.write(Path.of(args[2]),output.toByteArray());
 }
}
