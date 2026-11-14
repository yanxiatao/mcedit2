import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
/** 仅开放实际 CPU 属性入口及静态化天体计算；世界出生点和 fixedTime 是显式输入。无反射。 */
public class ItemWorldCpuAdapter implements Opcodes {
 public static void main(String[] args)throws Exception {
  var reader=new ClassReader(Files.readAllBytes(Path.of(args[0])));var output=new ClassWriter(0);var own=reader.getClassName();
  boolean clock=own.equals("dwy")||own.equals("dfk")||own.equals("aym");
  reader.accept(new ClassVisitor(ASM8,output){
   @Override public void visit(int version,int flags,String name,String signature,String parent,String[] interfaces){super.visit(version,(flags&~(ACC_PRIVATE|ACC_PROTECTED))|ACC_PUBLIC,name,signature,parent,interfaces);}
   @Override public void visitInnerClass(String name,String outer,String inner,int flags){super.visitInnerClass(name,outer,inner,(flags&~(ACC_PRIVATE|ACC_PROTECTED))|ACC_PUBLIC);}
   @Override public MethodVisitor visitMethod(int flags,String name,String desc,String signature,String[] exceptions){
    boolean convert=clock&&name.equals("a")&&desc.equals(own.equals("aym")?"(JF)F":"(J)F");
    var target=super.visitMethod((flags&~(ACC_PRIVATE|ACC_PROTECTED))|ACC_PUBLIC|(convert?ACC_STATIC:0),convert?"sample":name,convert?"(L"+own+";"+desc.substring(1):desc,signature,exceptions);
    return new MethodVisitor(ASM8,target){
     @Override public void visitAnnotableParameterCount(int count,boolean visible){super.visitAnnotableParameterCount(count+(convert?1:0),visible);}
     @Override public AnnotationVisitor visitParameterAnnotation(int index,String annotation,boolean visible){return super.visitParameterAnnotation(index+(convert?1:0),annotation,visible);}
     @Override public void visitFieldInsn(int opcode,String owner,String field,String descriptor){
      if(convert&&opcode==GETFIELD&&owner.equals(own)&&descriptor.equals("Ljava/util/OptionalLong;")){
       super.visitMethodInsn(INVOKESTATIC,"ItemWorldReference","fixedTime","(L"+own+";)Ljava/util/OptionalLong;",false);return;
      }
      super.visitFieldInsn(opcode,owner,field,descriptor);
     }
     @Override public void visitMethodInsn(int opcode,String owner,String method,String descriptor,boolean itf){
      if(own.equals("ahn$1")&&owner.equals("amu")&&method.equals("T")&&descriptor.equals("()Let;")){
       super.visitMethodInsn(INVOKESTATIC,"ItemWorldReference","spawn","(Lamu;)Let;",false);return;
      }
      super.visitMethodInsn(opcode,owner,method,descriptor,itf);
     }
    };
   }
  },0);
  Files.write(Path.of(args[1]),output.toByteArray());
 }
}
