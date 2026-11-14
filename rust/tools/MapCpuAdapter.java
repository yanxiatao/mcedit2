import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
/** 仅静态化分析副本 MapRenderer 的绘制入口；截获 GPU / sprite / 字体环境边界。 */
public final class MapCpuAdapter implements Opcodes {
 public static void main(String[] args)throws Exception {
  var reader=new ClassReader(Files.readAllBytes(Path.of(args[0])));var output=new ClassWriter(0);String own=reader.getClassName();
  reader.accept(new ClassVisitor(ASM8,output){
   @Override public void visitEnd(){
    if(own.equals("eqt")){
     var method=super.visitMethod(ACC_PUBLIC|ACC_STATIC,"project","(Leqt;Ljm;Ldcx;Ljava/lang/String;DDDLwz;)V",null,null);
     method.visitCode();for(int i=0;i<4;i++)method.visitVarInsn(ALOAD,i);
     method.visitVarInsn(DLOAD,4);method.visitVarInsn(DLOAD,6);method.visitVarInsn(DLOAD,8);method.visitVarInsn(ALOAD,10);
     method.visitMethodInsn(INVOKEVIRTUAL,"eqt","a","(Ljm;Ldcx;Ljava/lang/String;DDDLwz;)V",false);
     method.visitInsn(RETURN);method.visitMaxs(11,11);method.visitEnd();
    }
    super.visitEnd();
   }
   @Override public void visitInnerClass(String name,String outer,String inner,int flags){
    super.visitInnerClass(name,outer,inner,name.equals("fic$a")?(flags&~(ACC_PRIVATE|ACC_PROTECTED))|ACC_PUBLIC:flags);
   }
   @Override public MethodVisitor visitMethod(int flags,String name,String desc,String signature,String[] exceptions){
    if(!name.equals("a")||!desc.equals("(Lfbi;Lgez;ZI)V"))return super.visitMethod(flags,name,desc,signature,exceptions);
    var target=super.visitMethod(ACC_PUBLIC|ACC_STATIC,name,"(Lfic$a;"+desc.substring(1),null,exceptions);
    return new MethodVisitor(ASM8,target){
     @Override public void visitAnnotableParameterCount(int count,boolean visible){super.visitAnnotableParameterCount(count+1,visible);}
     @Override public AnnotationVisitor visitParameterAnnotation(int index,String desc,boolean visible){return super.visitParameterAnnotation(index+1,desc,visible);}
     @Override public void visitFieldInsn(int opcode,String owner,String field,String descriptor){
      if(opcode==GETFIELD&&(owner.equals("fic$a")||owner.equals("fic"))){
       super.visitInsn(POP);
       if(descriptor.equals("Leqt;"))super.visitFieldInsn(GETSTATIC,"MapReference","current","Leqt;");
       else super.visitInsn(descriptor.equals("Z")?ICONST_0:ACONST_NULL);
       return;
      }
      super.visitFieldInsn(opcode,owner,field,descriptor);
     }
     @Override public void visitMethodInsn(int opcode,String owner,String method,String d,boolean itf){
      if(owner.equals("fic$a")&&method.equals("b")&&d.equals("()V")){super.visitInsn(POP);return;}
      if(owner.equals("gri")&&d.equals("(Leqn;)Lgql;")){
       super.visitMethodInsn(INVOKESTATIC,"MapReference","sprite","(Lgri;Leqn;)Lgql;",false);return;
      }
      if(owner.equals("gql")){
       super.visitInsn(POP);
       if(d.equals("()F"))super.visitInsn(method.equals("c")||method.equals("g")?FCONST_0:FCONST_1);
       else if(d.equals("()Lakr;"))super.visitInsn(ACONST_NULL);
       else throw new IllegalStateException("未绑定的地图 sprite 方法 "+method+d);
       return;
      }
      if(owner.equals("gfh")&&d.equals("(Lakr;)Lgfh;")){super.visitInsn(POP);super.visitInsn(ACONST_NULL);return;}
      if(owner.equals("eqn")&&method.equals("g")){
       super.visitInsn(POP);super.visitMethodInsn(INVOKESTATIC,"java/util/Optional","empty","()Ljava/util/Optional;",false);return;
      }
      super.visitMethodInsn(opcode,owner,method,d,itf);
     }
    };
   }
  },0);
  Files.write(Path.of(args[1]),output.toByteArray());
 }
}
