import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
/** 仅在分析副本静态化生成器入口，并将 SpriteContents 的 CPU 像素边界接到给定帧。无反射。 */
public final class GeneratedItemCpuAdapter implements Opcodes {
 public static void main(String[] args)throws Exception {
  var output=new ClassWriter(0);
  var reader=new ClassReader(Files.readAllBytes(Path.of(args[0])));String own=reader.getClassName();
  reader.accept(new ClassVisitor(ASM8,output){
   @Override public MethodVisitor visitMethod(int flags,String name,String desc,String signature,String[] exceptions){
    boolean convert=(flags&ACC_STATIC)==0&&!name.equals("<init>");
    var target=super.visitMethod(convert?(flags&~(ACC_PRIVATE|ACC_PROTECTED))|ACC_PUBLIC|ACC_STATIC:flags,
      name,convert?"(L"+own+";"+desc.substring(1):desc,null,exceptions);
    return new MethodVisitor(ASM8,target){
     @Override public void visitAnnotableParameterCount(int count,boolean visible){super.visitAnnotableParameterCount(count+(convert?1:0),visible);}
     @Override public AnnotationVisitor visitParameterAnnotation(int parameter,String annotation,boolean visible){return super.visitParameterAnnotation(parameter+(convert?1:0),annotation,visible);}
     @Override public void visitMethodInsn(int opcode,String owner,String method,String descriptor,boolean itf){
      if(owner.equals(own)&&opcode!=INVOKESTATIC&&!method.equals("<init>")){
       super.visitMethodInsn(INVOKESTATIC,owner,method,"(L"+own+";"+descriptor.substring(1),false);return;
      }
      if(owner.equals("gqf")){
       String hook=switch(method+descriptor){case "a()I"->"width";case "b()I"->"height";
        case "d()Ljava/util/stream/IntStream;"->"frames";case "a(III)Z"->"transparent";
        default->throw new IllegalStateException("未绑定的像素边界 "+method+descriptor);};
       super.visitMethodInsn(INVOKESTATIC,"GeneratedItemReference",hook,"(Lgqf;"+descriptor.substring(1),false);return;
      }
      if(owner.equals("gql")&&own.equals("ggd")){
       String hook=switch(method+descriptor){case "k()F"->"shrink";case "a(F)F","c(F)F"->"uv";
        default->throw new IllegalStateException("未绑定的 sprite UV 边界 "+method+descriptor);};
       super.visitMethodInsn(INVOKESTATIC,"GeneratedItemReference",hook,"(Lgql;"+descriptor.substring(1),false);return;
      }
      super.visitMethodInsn(opcode,owner,method,descriptor,itf);
     }
     @Override public void visitInvokeDynamicInsn(String method,String descriptor,Handle bootstrap,Object... values){
      for(int i=0;i<values.length;i++)if(values[i] instanceof Handle h&&h.getOwner().equals(own)&&h.getTag()!=H_INVOKESTATIC){
       values[i]=new Handle(H_INVOKESTATIC,own,h.getName(),"(L"+own+";"+h.getDesc().substring(1),false);
      }
      super.visitInvokeDynamicInsn(method,descriptor,bootstrap,values);
     }
    };
   }
  },0);
  Files.write(Path.of(args[1]),output.toByteArray());
 }
}
