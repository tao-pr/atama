# Model serving Ollama.cpp

Following is the list of models for local serving.

## OpenAI-compat /chat/completions

GGUF quantized for CPU/GPU.

- `qwen3.5:0.8b`\
  Qwen 3.5 0.8B, a general-purpose model for text summarization and understanding.
- [`hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M`](https://huggingface.co/SparkX/SparkX2.6-1.7B-GGUF)\
  Spark X2.6 1.7B 4096 context (1.8GB), a general-purpose model for text generation, coding, multi-step planning, on-device inference, controlleable thinking budget.
- [`hf.co/Qwen/Qwen2.5-Coder-3B-Instruct-GGUF:Q4_K_M`](https://huggingface.co/Qwen/Qwen2.5-Coder-3B-Instruct-GGUF)\
  QWEN2.5 Coder 3B 4096 context (2.1GB), a model for coding and programming tasks.
- [`qwen3.5:4b`](https://ollama.com/library/qwen3.5:4b)\
  Qwen 3.5 4B, a general-purpose model for text summarization and coding with a better tool usage than Qwen 2.5 Coder.
- [`hf.co/AtomicChat/LFM3-1.7B-Uncensored-GGUF:Q4_K_M`](https://huggingface.co/AtomicChat/LFM3-1.7B-Uncensored-GGUF)\
  AtomicChat's LFM3-1.7B without prompt rejection.

## Text Embedding models

- [`hf.co/bartowski/granite-embedding-30m-english-GGUF:Q4_K_M`](https://huggingface.co/bartowski/granite-embedding-30m-english-GGUF)\
  30MB Granite Embeddings, a model for generating embeddings for text data.
