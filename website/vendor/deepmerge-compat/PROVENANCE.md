# Deepmerge compatibility

The starting point is `dist/cjs.js` from the published `deepmerge@4.3.1` archive with integrity `sha512-3sUqbMEc77XqpdNO7FRyRog+eW3ph+GYCbj+rK+uYyRMuwsVy0rMiVtPn+QJlKFvWP/1PYpapqYn0Me2knFn+A==`. The upstream MIT license is included verbatim.

The Svelte plugin imports the callable CommonJS default and uses its custom array replacement callback when merging local configuration. This package retains that behavior. It discards `__proto__` from either input, defines own data properties on the destination, and aborts if an input refuses a property lookup. These changes address the destination prototype mutation described in [GHSA-gcqx-w48m-w2h4](https://github.com/advisories/GHSA-gcqx-w48m-w2h4) and its [upstream report](https://github.com/TehShrike/deepmerge/issues/273).

This website reuses the implementation, license and fixture from UAR commit `dd09afc2a5477e778d630009b7e924de1cc2dc14` at `frontend/vendor/deepmerge-compat`. The website declares the local package as a root development dependency so that npm can link it from the website directory. A scoped override references that declaration for the dependency of `@svgr/plugin-svgo@8.1.0`. The package version identifies a private compatibility package and carries no upstream release claim.
