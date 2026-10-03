import {
    assemble, compileSource, decompile, disassemble, generateAbi,
    parseArgument, runProgram, Simulator,
} from '../pkg/silex_cli.js';

function assert(condition, message) {
    if (!condition) throw new Error(message);
}

async function rejects(call, message) {
    try {
        await call();
    } catch (error) {
        assert(error instanceof Error, 'Failures should be JavaScript Error objects');
        return;
    }
    throw new Error(message);
}

export async function runTests() {
    const source = 'entry main(n: u64) -> u64 { return n * 2u64; }';
    const bytes = compileSource(source);
    const options = `{"arguments":[${parseArgument('21')}]}`;
    const answer = JSON.parse(runProgram(bytes, options));
    assert(bytes instanceof Uint8Array, 'Compiled modules should be Uint8Array');
    assert(answer.value.value === '42', 'Standalone program should return 42');
    const assembly = disassemble(bytes);
    assert(assembly.includes('MUL'), 'Disassembly should contain program instructions');
    const rebuilt = assemble(assembly);
    assert(rebuilt instanceof Uint8Array, 'Assembler should produce module bytes');
    assert(JSON.stringify(JSON.parse(runProgram(rebuilt, options))) === JSON.stringify(answer), 'Assembly round trip must preserve execution');
    assert(JSON.stringify(Array.from(rebuilt)) === JSON.stringify(Array.from(bytes)), 'Assembly must retain all module metadata');
    assert(JSON.parse(runProgram(compileSource(decompile(bytes)), options)).value.value === '42', 'Decompiled source must execute correctly');
    assert(decompile(bytes).includes('2u64'), 'Decompilation should recover the multiplier');
    assert(JSON.parse(generateAbi(source)).data[0].name === 'main', 'ABI should describe the entry');
    const large = '18446744073709551615';
    assert(JSON.parse(parseArgument(large)).value.value === large, 'Large values retain precision');
    await rejects(() => compileSource('entry {'), 'Invalid source must throw');
    await rejects(() => disassemble(new Uint8Array()), 'Invalid module must throw');
    await rejects(() => runProgram(bytes, '{'), 'Invalid options must throw');
    await rejects(() => runProgram(bytes), 'Missing arguments must throw');
    await rejects(() => new Simulator('{"unknown":true}'), 'Invalid snapshot must throw');

    const hash = '01'.repeat(32);
    const counter = `
        entry increment() {
            let s = Storage::new();
            let n: u64 = s.load(b"count").unwrap_or(0u64);
            s.store(b"count", n + 1u64);
            return 0;
        }
        entry fail() {
            Storage::new().store(b"count", 99u64);
            return 1;
        }
    `;
    const simulator = new Simulator();
    let restored;
    try {
        const first = JSON.parse(await simulator.runSource(hash, counter));
        assert(first.execution.exit_value.value === 0, 'Initial contract should succeed');
        assert(Array.isArray(first.logs), 'Results include contract logs');
        simulator.register('02'.repeat(32), compileSource('entry main() { return 0; }'));
        restored = new Simulator(simulator.storageJson());
        const second = JSON.parse(await restored.run(hash));
        assert(second.execution.exit_value.value === 0, 'Restored contract should succeed');
        const before = restored.storageJson();
        const failed = JSON.parse(await restored.run(hash, '{"entry":1}'));
        assert(failed.execution.exit_value.value === 1, 'Contract failures resolve with an exit code');
        assert(restored.storageJson() === before, 'Failed writes must roll back');
        const snapshot = JSON.parse(before);
        assert(snapshot.contracts[hash].data[0].value.value.value === '2', 'State persists across calls');
        const exhausted = JSON.parse(await restored.run(hash, '{"gas_limit":0}'));
        assert(exhausted.execution.exit_value.type === 'Error', 'Gas exhaustion is a contract error');
        await rejects(() => restored.run('invalid'), 'Invalid hashes must reject');
        await rejects(() => restored.run(hash, '{"unknown":true}'), 'Unknown options must reject');
        await rejects(() => restored.run('ff'.repeat(32)), 'Unregistered contracts must reject');
    } finally {
        restored?.free();
        simulator.free();
    }
}
