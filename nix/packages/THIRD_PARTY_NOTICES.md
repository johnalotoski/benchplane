# Third-party notices for the packaged inference runtime

## llama.cpp

Benchplane's fixed CPU inference helper links against llama.cpp version 10133 from
the pinned Nixpkgs revision. Upstream source: <https://github.com/ggml-org/llama.cpp>.

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## SmolLM2-135M-Instruct Q2_K GGUF

The package contains the 88,201,792-byte
`SmolLM2-135M-Instruct.Q2_K.gguf` conversion published by QuantFactory from
HuggingFaceTB's SmolLM2-135M-Instruct model. The model is licensed under
Apache-2.0, the same license whose full text is installed with Benchplane.

- Fixed source revision: `c33bd7b3a0c1c5048af630f0198eb2a29977b422`
- Source: <https://huggingface.co/QuantFactory/SmolLM2-135M-Instruct-GGUF>
- Original model: <https://huggingface.co/HuggingFaceTB/SmolLM2-135M-Instruct>
- SHA-256: `55aa88ddac43adce6af0e9be8d6cdff2337a3835cd9b50bbcd7a894eb66dfc75`

Benchplane does not claim authorship of llama.cpp or the model fixture.
