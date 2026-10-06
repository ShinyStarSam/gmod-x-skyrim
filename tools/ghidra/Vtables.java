// List the function tables ("vftables", found through MSVC's class-name info) of every class whose
// name contains one of the texts given, each slot's function, and those functions decompiled.
// Read-only; output goes to the path given as the first script argument.
//   tools\ghidra_dump.ps1 -Script Vtables -Out build\ghidra\vt.txt -Offsets CPhysicsEnvironment
// A text starting with "=" must match the class name exactly ("=CPhysicsObject"). A "0x..." offset
// is read as a function table at that offset (for DLLs without class-name info).
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class Vtables extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        PrintWriter out = new PrintWriter(new FileWriter(args[0]));
        long base = currentProgram.getImageBase().getOffset();
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        Set<Function> done = new HashSet<>();
        for (int i = 1; i < args.length; i++) {
            if (args[i].startsWith("0x")) {
                dump(out, di, done, base, toAddr(base + Long.decode(args[i])), "table");
                continue;
            }
            boolean exact = args[i].startsWith("=");
            String want = exact ? args[i].substring(1) : args[i];
            for (Symbol s : currentProgram.getSymbolTable().getAllSymbols(true)) {
                if (!s.getName().equals("vftable")) continue;
                String cls = s.getParentNamespace().getName(true);
                if (exact ? !s.getParentNamespace().getName().equals(want) : !cls.contains(want)) continue;
                dump(out, di, done, base, s.getAddress(), cls + "::vftable");
            }
        }
        out.close();
    }

    void dump(PrintWriter out, DecompInterface di, Set<Function> done, long base, Address t, String name)
            throws Exception {
        out.printf("#################### %s at +0x%x ####################%n", name, t.getOffset() - base);
        List<Function> slots = new ArrayList<>();
        for (int k = 0; k < 400; k++) {
            Address p = t.add(8L * k);
            if (k > 0 && getSymbolAt(p) != null && getSymbolAt(p).getName().equals("vftable")) break;
            long v = getLong(p);
            Address va = toAddr(v);
            if (currentProgram.getMemory().getBlock(va) == null || !currentProgram.getMemory().getBlock(va).isExecute()) break;
            Function f = getFunctionAt(va);
            if (f == null) { disassemble(va); f = createFunction(va, null); } // in memory only: read-only project
            if (f == null) break;
            out.printf("  slot %3d (+0x%03x) -> +0x%x %s (%d bytes)%n", k, 8 * k, v - base, f.getName(),
                f.getBody().getNumAddresses());
            slots.add(f);
        }
        for (Function f : slots) {
            if (done.contains(f)) continue;
            done.add(f);
            out.printf("-------- +0x%x %s --------%n", f.getEntryPoint().getOffset() - base, f.getName());
            DecompileResults res = di.decompileFunction(f, 120, monitor);
            out.println(res.decompileCompleted() ? res.getDecompiledFunction().getC() : "FAILED " + res.getErrorMessage());
        }
    }
}
