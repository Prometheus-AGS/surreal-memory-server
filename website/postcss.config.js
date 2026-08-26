// Shadow the repo-root postcss.config.cjs (which loads the Tailwind plugin for
// the app frontend) so the Docusaurus build doesn't try to resolve tailwindcss
// from website/node_modules. Docusaurus applies autoprefixer via its own build
// pipeline, so an empty plugin set here is sufficient.
module.exports = { plugins: {} };
