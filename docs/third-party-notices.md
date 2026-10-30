# Third-party notices

## OpenCode

The standalone text-generation boundary and Claude process options in
`src/ai/provider.rs` adapt OpenCode's utility-call strategy. Upstream sources
at commit `bf5d12c588911a35ec56ad991c3b83cf937f424a`:

- [Generate.text](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/generate.ts)
- [Claude utility calls](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/plugin/provider/claude-agent/language-model/claude-agent-language-model.ts)
- [Utility system prompt](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/plugin/provider/claude-agent/language-model/system-prompt.ts)
- [ClaudeQueryOptions](https://github.com/anomalyco/opencode/blob/bf5d12c588911a35ec56ad991c3b83cf937f424a/packages/core/src/plugin/provider/claude-agent/claude/query-options.ts)

The TypeScript strategy is adapted to Rust and native CLI interfaces for
one-shot commit generation. OpenCode is licensed under the following MIT license:

MIT License

Copyright (c) 2025 opencode

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


## T3 Code

The structured commit-generation prompt, schema, cleanup, Codex/Claude invocation
patterns, and dedicated Antigravity helper are adapted to Rust from T3 Code at
commit `bf270e74e4c6144fc65648186e879d8bfdb411ed`. The ACP transport is implemented for this app’s single-request
flow; this does not import T3’s full chat orchestrator or runtime installer.

- [textGenerationPrompts.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/packages/provider-core/src/server/textGenerationPrompts.ts)
- [textGenerationOperations.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/packages/provider-core/src/server/textGenerationOperations.ts)
- [textGenerationUtils.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/packages/provider-core/src/server/textGenerationUtils.ts)
- [CodexTextGeneration.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/apps/server/src/textGeneration/CodexTextGeneration.ts)
- [ClaudeTextGeneration.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/apps/server/src/textGeneration/ClaudeTextGeneration.ts)
- [AntigravityTextGeneration.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/apps/server/src/textGeneration/AntigravityTextGeneration.ts)
- [antigravityAuthSupport.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/apps/server/src/provider/antigravityAuthSupport.ts)
- [AntigravitySessionFiles.ts](https://github.com/pingdotgg/t3code/blob/bf270e74e4c6144fc65648186e879d8bfdb411ed/apps/server/src/provider/acp/AntigravitySessionFiles.ts)

MIT License

Copyright (c) 2026 T3 Tools Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
