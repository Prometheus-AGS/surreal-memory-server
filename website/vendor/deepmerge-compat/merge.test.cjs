const assert = require('node:assert/strict');
const { test } = require('node:test');
const merge = require('./index.cjs');

test('Svelte option merges preserve nested values and replace arrays', () => {
	const left = { compilerOptions: { dev: true, namespaces: ['html'] } };
	const right = { compilerOptions: { css: 'external', namespaces: ['svg'] } };

	assert.deepEqual(merge(left, right, { arrayMerge: (target, source) => source ?? target }), {
		compilerOptions: { dev: true, css: 'external', namespaces: ['svg'] },
	});
	assert.deepEqual(left.compilerOptions.namespaces, ['html']);
	assert.deepEqual(merge({ compilerOptions: null }, { compilerOptions: { dev: true } }), {
		compilerOptions: { dev: true },
	});
});

test('own prototype keys cannot mutate the merged object prototype', () => {
	const unsafe = JSON.parse('{"__proto__":{"polluted":true},"preserved":1}');
	for (const value of [merge(unsafe, {}), merge(Object.create(null), unsafe)]) {
		assert.equal(Object.getPrototypeOf(value), Object.prototype);
		assert.equal(Object.hasOwn(value, '__proto__'), false);
		assert.equal(value.polluted, undefined);
		assert.equal(value.preserved, 1);
	}
});

test('a throwing property lookup aborts the merge', () => {
	const unsafe = new Proxy({}, { has() { throw new Error('unsafe lookup'); } });
	assert.throws(() => merge(unsafe, { nested: true }), /unsafe lookup/);
});
