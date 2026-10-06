// Dump the functions on vphysics.dll's crash chain: disassembly around each address and the
// decompiled function. Read-only; output goes to the path given as the first script argument.
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class ChainDump extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        PrintWriter out = new PrintWriter(new FileWriter(args[0]));
        long base = currentProgram.getImageBase().getOffset();
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        Set<Function> done = new HashSet<>();
        for (int i = 1; i < args.length; i++) {
            long off = Long.decode(args[i]);
            Address a = toAddr(base + off);
            Function f = getFunctionContaining(a);
            out.printf("==================== +0x%x (%s) in %s ====================%n", off, a,
                f == null ? "NO FUNCTION" : f.getName() + " @ +0x" + Long.toHexString(f.getEntryPoint().getOffset() - base));
            // instructions around the address
            Instruction ins = getInstructionContaining(a);
            for (int k = 0; k < 14 && ins != null && ins.getPrevious() != null; k++) ins = ins.getPrevious();
            for (int k = 0; k < 24 && ins != null; k++, ins = ins.getNext()) {
                out.printf("%s +0x%x  %s%n", ins.getAddress().equals(a) || ins.contains(a) ? ">>" : "  ",
                    ins.getAddress().getOffset() - base, ins);
            }
            if (f == null || done.contains(f)) { out.println("(function already printed)"); continue; }
            done.add(f);
            out.println("-- callers:");
            for (Reference r : getReferencesTo(f.getEntryPoint())) {
                Function c = getFunctionContaining(r.getFromAddress());
                out.printf("   +0x%x %s%n", r.getFromAddress().getOffset() - base, c == null ? "?" : c.getName());
            }
            DecompileResults res = di.decompileFunction(f, 120, monitor);
            out.println("-- decompiled:");
            out.println(res.decompileCompleted() ? res.getDecompiledFunction().getC() : "FAILED " + res.getErrorMessage());
        }
        out.close();
    }
}
