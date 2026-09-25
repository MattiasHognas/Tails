# Fusion experiment: explain output of every miss

Generated from the `scripts/e2e.sh` logs of [fusion-containers.md](fusion-containers.md).
For each model and question that some configuration did not rank first: the dense and
keyword top 5 (identical for every fusion; the keyword list depends on stopwords), then
per configuration that missed it the fused top 5 (normalized score) and the reranked
sources (score after kind prior and recency weight).

## BAAI/bge-small-en-v1.5

### q17-inventory-logs-by-service (must retrieve ["logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e"]): "Which errors did the inventory service log yesterday?"

Dense and keyword search (nostop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.8387  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8103  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7399  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.7392  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.7308  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 8.1636  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    2. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    3. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6111  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.5833  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3750  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3667  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.817
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.599
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.572
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.359
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.368
```

`dbsf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8799  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8112  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7329  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4996  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4983  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.795
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.718
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.488
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.490
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5625  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.4167  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.2381  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.2250  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.735
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.551
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.408
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.233
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.220
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7738  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7083  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5903  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.5844  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.898
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.758
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.694
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.578
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.573
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8712  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7941  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.7418  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.7292  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.778
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.727
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.715
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9757  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9478  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.9449  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.9384  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.972
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.956
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.929
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.926
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.920
```

`rrf(k=2,w=1:1.5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8442  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6465  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6061  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3916  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3877  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.827
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.634
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.594
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.384
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.380
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6286  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4163  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4048  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.653
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.616
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.408
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.397
```

`rrf(k=2,w=1:3) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6889  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4600  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4400  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.675
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.451
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.431
```

`rrf(k=2,w=1.5:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6169  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5981  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4167  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3994  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.832
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.605
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.586
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.408
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.391
```

`rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6429  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6000  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4286  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.630
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.588
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.441
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.420
```

`rrf(k=2,w=3:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8667  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6800  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6182  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5000  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4762  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.849
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.666
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.606
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.490
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.467
```

Dense and keyword search (stop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.8387  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8103  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7399  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.7392  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.7308  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4167  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.4000  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3409  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.408
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.392
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.334
```

`dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9541  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8796  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5069  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5057  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4901  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.862
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.497
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.496
    [DOC #5] Log: checkout-api - error - PaymentDeclinedException: issuer declined card on retry # of # (Logs)  score 0.475
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5000  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.2667  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.2500  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.2167  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.490
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.261
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.245
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.212
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8333  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6349  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.6250  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.5357  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.817
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.622
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.613
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.525
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9091  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7738  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.7692  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.6798  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.891
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.758
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.754
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.666
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9836  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9526  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.9524  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.9233  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.964
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.934
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.933
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.900
```

`rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4371  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.4298  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3857  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.428
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.421
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.378
```

`rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4592  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.4571  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4208  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.450
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.448
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.412
```

`rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5029  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.5000  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4727  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.490
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.493
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.463
```

`rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4545  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.4298  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3458  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.445
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.421
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.339
```

`rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4857  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.4571  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3571  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.476
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.448
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.350
```

`rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5333  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5029  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4000  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.523
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.493
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.377
```

### q25-failing-right-now (must retrieve ["logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763"]): "What is failing in notifications right now?"

Dense and keyword search (nostop):

```
  dense search for "What is failing in notifications right now?":
    1. 0.7628  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.7364  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "What is failing in notifications right now?":
    1. 22.4156  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 14.6065  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  dense search for "notifications delivery failing errors":
    1. 0.8101  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.8063  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "notifications delivery failing errors":
    1. 21.6545  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
    2. 21.4897  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
```

`rrf(k=1) nostop`, `rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8750  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.6250  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired (Logs)  score 0.650
    [DOC #2] Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay (Logs)  score 0.611
```

Dense and keyword search (stop):

```
  dense search for "What is failing in notifications right now?":
    1. 0.7628  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.7364  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "What is failing in notifications right now?":
    1. 22.4156  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 14.6065  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  dense search for "notifications delivery failing errors":
    1. 0.8101  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.8063  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "notifications delivery failing errors":
    1. 21.6545  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
    2. 21.4897  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
```

### q40-checkout-outage-root-cause (must retrieve ["incident_inc-checkout-outage"]): "What was the root cause of the last checkout outage?"

Dense and keyword search (nostop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.7303  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.7237  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.6977  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 24.6248  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.8333  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5000  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.806
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.688
    [DOC #3] Checkout errors after deploy (Incident)  score 0.413
```

`rrf(k=1) nostop`, `rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.7500  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.3333  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.725
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.619
    [DOC #3] Checkout errors after deploy (Incident)  score 0.275
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9167  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.7143  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.886
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.756
    [DOC #3] Checkout errors after deploy (Incident)  score 0.589
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.9545  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.8333  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.923
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.788
    [DOC #3] Checkout errors after deploy (Incident)  score 0.688
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.9918  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.9677  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.959
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.818
    [DOC #3] Checkout errors after deploy (Incident)  score 0.799
```

`rrf(k=2,w=1:1.5) nostop`, `rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8442  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.5303  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.816
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.700
    [DOC #3] Checkout errors after deploy (Incident)  score 0.438
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8571  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.5571  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.829
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.707
    [DOC #3] Checkout errors after deploy (Incident)  score 0.460
```

`rrf(k=2,w=1:3) nostop`, `rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.8667  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.6000  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.851
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.715
    [DOC #3] Checkout errors after deploy (Incident)  score 0.495
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.8442  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5303  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.820
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.696
    [DOC #3] Checkout errors after deploy (Incident)  score 0.438
```

`rrf(k=2,w=2:1) nostop`, `rrf(k=2,w=1:2) stop`, `rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.8571  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5571  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.829
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.707
    [DOC #3] Checkout errors after deploy (Incident)  score 0.460
```

`rrf(k=2,w=3:1) nostop`, `rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8667  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.6000  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.838
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.726
    [DOC #3] Checkout errors after deploy (Incident)  score 0.495
```

Dense and keyword search (stop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.7303  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.7237  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.6977  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 18.8772  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8333  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.5000  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.806
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.688
    [DOC #3] Checkout errors after deploy (Incident)  score 0.413
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  incident_inc-checkout-latency  Checkout latency degradation
    2. 0.9167  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.7143  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.886
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.756
    [DOC #3] Checkout errors after deploy (Incident)  score 0.589
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9918  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.9677  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.959
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.818
    [DOC #3] Checkout errors after deploy (Incident)  score 0.799
```

### q41-gateway-postmortem-actions (must retrieve ["incident_inc-gateway-cert"]): "Which action items came out of the postmortem on failed card payments at the bank gateway?"

Dense and keyword search (nostop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6676  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.6279  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 39.8123  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf(k=5) nostop`, `rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9091  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.987
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9836  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.068
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

Dense and keyword search (stop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6676  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.6279  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 30.8881  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

### q50-checkout-owner-runbook (must retrieve ["catalog_checkout"]): "Who owns checkout and where is its runbook?"

Dense and keyword search (nostop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6657  catalog_checkout  Service: checkout
    2. 0.5968  slo_slo-checkout-avail  Checkout availability
    3. 0.5724  dashboard_dash-checkout  Checkout overview
    4. 0.5585  logpattern_d440cd4c56b8f63b1c70bc320210dfa2_2026-03-09  Log: checkout - warn - slow query on orders table (#ms)
    5. 0.5553  change_dep-co-1  Deployed checkout 2.14.0 to prod
  keyword search for "Who owns checkout and where is its runbook?":
    1. 8.6347  monitor_1001  Checkout p95 latency above 1.5s
    2. 8.5864  catalog_checkout  Service: checkout
    3. 5.8509  monitor_1002  Checkout p95 latency above 1.5s (staging)
    4. 4.4920  dashboard_dash-checkout  Checkout overview
    5. 4.0930  incident_inc-checkout-outage  Orders stuck at the payment step
  dense search for "checkout owner team contacts runbook":
    1. 0.7143  catalog_checkout  Service: checkout
    2. 0.6116  slo_slo-checkout-avail  Checkout availability
    3. 0.5984  dashboard_dash-checkout  Checkout overview
    4. 0.5917  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.5885  logpattern_d440cd4c56b8f63b1c70bc320210dfa2_2026-03-09  Log: checkout - warn - slow query on orders table (#ms)
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9959  catalog_checkout  Service: checkout
    2. 0.9679  dashboard_dash-checkout  Checkout overview
    3. 0.9331  slo_slo-checkout-avail  Checkout availability
    4. 0.8713  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-29  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.8497  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-27  Log: checkout - error - checkout tax service returned # for region SE after #ms
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.821
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.747
    [DOC #3] Checkout availability (SLO)  score 0.721
    [DOC #4] Checkout overview (Dashboard)  score 0.726
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.653
```

Dense and keyword search (stop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6657  catalog_checkout  Service: checkout
    2. 0.5968  slo_slo-checkout-avail  Checkout availability
    3. 0.5724  dashboard_dash-checkout  Checkout overview
    4. 0.5585  logpattern_d440cd4c56b8f63b1c70bc320210dfa2_2026-03-09  Log: checkout - warn - slow query on orders table (#ms)
    5. 0.5553  change_dep-co-1  Deployed checkout 2.14.0 to prod
  keyword search for "Who owns checkout and where is its runbook?":
    1. 5.9769  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
  dense search for "checkout owner team contacts runbook":
    1. 0.7143  catalog_checkout  Service: checkout
    2. 0.6116  slo_slo-checkout-avail  Checkout availability
    3. 0.5984  dashboard_dash-checkout  Checkout overview
    4. 0.5917  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.5885  logpattern_d440cd4c56b8f63b1c70bc320210dfa2_2026-03-09  Log: checkout - warn - slow query on orders table (#ms)
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  catalog_checkout  Service: checkout
    2. 0.9757  dashboard_dash-checkout  Checkout overview
    3. 0.9396  slo_slo-checkout-avail  Checkout availability
    4. 0.8713  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-29  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.8598  incident_inc-checkout-latency  Checkout latency degradation
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.831
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.750
    [DOC #3] Checkout availability (SLO)  score 0.726
    [DOC #4] Checkout overview (Dashboard)  score 0.732
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.655
```

## BAAI/bge-base-en-v1.5

### q17-inventory-logs-by-service (must retrieve ["logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e"]): "Which errors did the inventory service log yesterday?"

Dense and keyword search (nostop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.7165  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6949  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6070  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5932  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5905  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 8.1636  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    2. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    3. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6250  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.5833  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3929  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3667  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.817
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.613
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.572
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.385
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.359
```

`dbsf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8791  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8306  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7189  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5192  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4989  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.814
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.705
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.509
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.489
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5714  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.4167  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.2500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.2250  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.735
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.560
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.408
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.245
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.220
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7738  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7273  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.6071  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5903  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.898
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.758
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.713
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.595
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.578
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8712  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8125  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.7500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.7418  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.796
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.735
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.727
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9757  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9545  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.9454  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.9449  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.972
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.956
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.935
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.926
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.927
```

`rrf(k=2,w=1:1.5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8442  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6591  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6061  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4091  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3994  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.827
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.646
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.594
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.401
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.391
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6786  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6286  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4286  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4286  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.665
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.616
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.420
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.420
```

`rrf(k=2,w=1:3) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7000  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4762  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4667  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.686
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.467
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.457
```

`rrf(k=2,w=1.5:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6169  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6150  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4329  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3916  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.832
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.605
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.603
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.424
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.384
```

`rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6429  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6190  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4653  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4163  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.630
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.607
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.456
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.408
```

`rrf(k=2,w=3:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8667  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6800  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6400  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5143  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4600  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.849
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.666
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.627
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.504
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.451
```

Dense and keyword search (stop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.7165  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6949  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6070  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5932  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5905  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
    5. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4167  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3929  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3429  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.408
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.385
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.336
```

`dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9533  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8991  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5265  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5061  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5028  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.934
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.881
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.516
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.496
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.493
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5000  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.2667  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.2500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.2083  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.490
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.261
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.245
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.204
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8333  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6349  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.6071  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5625  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.817
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.622
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.595
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.551
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9091  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7738  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.7500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.7179  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.891
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.758
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.735
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.704
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9836  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9526  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.9454  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.9377  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.964
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.927
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.934
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.919
```

`rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4545  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4091  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3778  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.445
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.401
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.370
```

`rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4857  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4286  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4082  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.476
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.420
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.400
```

`rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5333  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4667  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4571  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.523
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.457
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.448
```

`rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4371  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4329  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3636  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.424
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.428
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.356
```

`rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4653  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.4592  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3929  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.456
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.450
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.385
```

`rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5143  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5000  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4429  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.504
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.490
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.434
```

### q30-exact-error-code (must retrieve ["logpattern_3d136cbffb285fecc3959b456619fdee"]): "Why does edge-gateway log ERR_CONN_RESET when calling payments upstream?"

Dense and keyword search (nostop):

```
  dense search for "Why does edge-gateway log ERR_CONN_RESET when calling payments upstream?":
    1. 0.8226  logpattern_a61d00036f005ca3d372d572df6ef8e7_2026-03-11  Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets
    2. 0.7908  logpattern_3d136cbffb285fecc3959b456619fdee_2026-03-11  Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments
    3. 0.7820  logpattern_4705b86a8dfbea737bd8a60cca937def_2026-03-11  Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms
    4. 0.7765  logpattern_6508759e327b0e2d631cc73d09684d56_2026-03-11  Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY
    5. 0.7430  logpattern_77b58135d001e28fd293fa9a1f17195b_2026-03-11  Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired
  keyword search for "Why does edge-gateway log ERR_CONN_RESET when calling payments upstream?":
    1. 44.2464  logpattern_3d136cbffb285fecc3959b456619fdee_2026-03-11  Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments
    2. 37.9122  logpattern_a61d00036f005ca3d372d572df6ef8e7_2026-03-11  Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets
    3. 34.8275  logpattern_6508759e327b0e2d631cc73d09684d56_2026-03-11  Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY
    4. 34.6302  logpattern_4705b86a8dfbea737bd8a60cca937def_2026-03-11  Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms
    5. 27.7378  logpattern_77b58135d001e28fd293fa9a1f17195b_2026-03-11  Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired
```

`rrf(k=2,w=1:3) nostop`, `rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_a61d00036f005ca3d372d572df6ef8e7_2026-03-11  Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets
    2. 0.8667  logpattern_3d136cbffb285fecc3959b456619fdee_2026-03-11  Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments
    3. 0.5600  logpattern_6508759e327b0e2d631cc73d09684d56_2026-03-11  Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY
    4. 0.5429  logpattern_4705b86a8dfbea737bd8a60cca937def_2026-03-11  Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms
    5. 0.4333  logpattern_77b58135d001e28fd293fa9a1f17195b_2026-03-11  Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets (Logs)  score 0.764
    [DOC #2] Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments (Logs)  score 0.753
    [DOC #3] Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY (Logs)  score 0.486
    [DOC #4] Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms (Logs)  score 0.472
    [DOC #5] Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired (Logs)  score 0.376
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_a61d00036f005ca3d372d572df6ef8e7_2026-03-11  Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets
    2. 0.8442  logpattern_3d136cbffb285fecc3959b456619fdee_2026-03-11  Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments
    3. 0.4848  logpattern_4705b86a8dfbea737bd8a60cca937def_2026-03-11  Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms
    4. 0.4752  logpattern_6508759e327b0e2d631cc73d09684d56_2026-03-11  Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY
    5. 0.3613  logpattern_77b58135d001e28fd293fa9a1f17195b_2026-03-11  Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets (Logs)  score 0.737
    [DOC #2] Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments (Logs)  score 0.733
    [DOC #3] Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms (Logs)  score 0.421
    [DOC #4] Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY (Logs)  score 0.413
    [DOC #5] Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired (Logs)  score 0.314
```

Dense and keyword search (stop):

```
  dense search for "Why does edge-gateway log ERR_CONN_RESET when calling payments upstream?":
    1. 0.8226  logpattern_a61d00036f005ca3d372d572df6ef8e7_2026-03-11  Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets
    2. 0.7908  logpattern_3d136cbffb285fecc3959b456619fdee_2026-03-11  Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments
    3. 0.7820  logpattern_4705b86a8dfbea737bd8a60cca937def_2026-03-11  Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms
    4. 0.7765  logpattern_6508759e327b0e2d631cc73d09684d56_2026-03-11  Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY
    5. 0.7430  logpattern_77b58135d001e28fd293fa9a1f17195b_2026-03-11  Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired
  keyword search for "Why does edge-gateway log ERR_CONN_RESET when calling payments upstream?":
    1. 44.2464  logpattern_3d136cbffb285fecc3959b456619fdee_2026-03-11  Log: edge-gateway - error - ERR_CONN_RESET reading response body from payments
    2. 37.9122  logpattern_a61d00036f005ca3d372d572df6ef8e7_2026-03-11  Log: edge-gateway - error - conn pool reset after ERR_POOL_EXHAUSTED; reconnecting upstream sockets
    3. 34.8275  logpattern_6508759e327b0e2d631cc73d09684d56_2026-03-11  Log: edge-gateway - error - calling payments upstream returned #: ERR_BAD_GATEWAY
    4. 34.6302  logpattern_4705b86a8dfbea737bd8a60cca937def_2026-03-11  Log: edge-gateway - error - upstream timeout calling payments: ERR_UPSTREAM_TIMEOUT after #ms
    5. 27.7378  logpattern_77b58135d001e28fd293fa9a1f17195b_2026-03-11  Log: edge-gateway - error - ERR_TLS_HANDSHAKE_FAILED with payments upstream: certificate expired
```

### q40-checkout-outage-root-cause (must retrieve ["incident_inc-checkout-outage"]): "What was the root cause of the last checkout outage?"

Dense and keyword search (nostop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.6426  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.6140  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.6077  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 24.6248  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9091  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8333  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.879
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.688
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9836  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.9677  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.951
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.799
```

Dense and keyword search (stop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.6426  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.6140  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.6077  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 18.8772  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

### q41-gateway-postmortem-actions (must retrieve ["incident_inc-gateway-cert"]): "Which action items came out of the postmortem on failed card payments at the bank gateway?"

Dense and keyword search (nostop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6040  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.5458  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 39.8123  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf(k=5) nostop`, `rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9091  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.987
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9836  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.068
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

Dense and keyword search (stop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6040  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.5458  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 30.8881  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

### q50-checkout-owner-runbook (must retrieve ["catalog_checkout"]): "Who owns checkout and where is its runbook?"

Dense and keyword search (nostop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6302  catalog_checkout  Service: checkout
    2. 0.5489  change_dep-co-1  Deployed checkout 2.14.0 to prod
    3. 0.5446  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.5375  change_dep-co-stg  Deployed checkout 2.14.0 to staging
    5. 0.5362  dashboard_dash-checkout  Checkout overview
  keyword search for "Who owns checkout and where is its runbook?":
    1. 8.6347  monitor_1001  Checkout p95 latency above 1.5s
    2. 8.5864  catalog_checkout  Service: checkout
    3. 5.8509  monitor_1002  Checkout p95 latency above 1.5s (staging)
    4. 4.4920  dashboard_dash-checkout  Checkout overview
    5. 4.0930  incident_inc-checkout-outage  Orders stuck at the payment step
  dense search for "checkout owner team contacts runbook":
    1. 0.6734  catalog_checkout  Service: checkout
    2. 0.5467  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    3. 0.5461  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.5446  logpattern_78b9e13a0700a5e105eab795055a15d7_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5241  dashboard_dash-checkout  Checkout overview
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9959  catalog_checkout  Service: checkout
    2. 0.9527  dashboard_dash-checkout  Checkout overview
    3. 0.9239  incident_inc-checkout-latency  Checkout latency degradation
    4. 0.8868  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.8793  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.893
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.747
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.682
    [DOC #4] Checkout overview (Dashboard)  score 0.715
    [DOC #5] Orders stuck at the payment step (Incident)  score 0.732
```

Dense and keyword search (stop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6302  catalog_checkout  Service: checkout
    2. 0.5489  change_dep-co-1  Deployed checkout 2.14.0 to prod
    3. 0.5446  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.5375  change_dep-co-stg  Deployed checkout 2.14.0 to staging
    5. 0.5362  dashboard_dash-checkout  Checkout overview
  keyword search for "Who owns checkout and where is its runbook?":
    1. 5.9769  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
  dense search for "checkout owner team contacts runbook":
    1. 0.6734  catalog_checkout  Service: checkout
    2. 0.5467  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    3. 0.5461  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.5446  logpattern_78b9e13a0700a5e105eab795055a15d7_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5241  dashboard_dash-checkout  Checkout overview
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  catalog_checkout  Service: checkout
    2. 0.9606  dashboard_dash-checkout  Checkout overview
    3. 0.9350  incident_inc-checkout-latency  Checkout latency degradation
    4. 0.8862  incident_inc-checkout-deploy  Checkout errors after deploy
    5. 0.8460  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.904
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.750
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.684
    [DOC #4] Checkout overview (Dashboard)  score 0.720
    [DOC #5] Checkout latency degradation in staging (Incident)  score 0.819
```

## BAAI/bge-large-en-v1.5

### q17-inventory-logs-by-service (must retrieve ["logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e"]): "Which errors did the inventory service log yesterday?"

Dense and keyword search (nostop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.7594  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7287  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6543  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.6369  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.6338  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 8.1636  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    2. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    3. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6250  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.5833  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4000  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3929  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.817
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.613
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.572
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.392
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.385
```

`dbsf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8823  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8200  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7239  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5264  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5010  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.865
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.804
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.709
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.516
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.491
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5714  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.4167  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.2500  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.2500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.735
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.560
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.408
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.245
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.245
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7738  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7273  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.6250  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.6071  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.898
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.758
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.713
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.613
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.595
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8712  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8125  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.7692  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.7500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.796
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.754
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.735
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9757  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9545  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.9524  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.9454  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.972
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.956
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.935
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.933
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.919
```

`rrf(k=2,w=1:1.5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8442  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6591  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6061  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4298  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4091  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.827
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.646
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.594
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.421
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.401
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6786  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6286  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4571  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4286  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.665
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.616
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.448
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.420
```

`rrf(k=2,w=1:3) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7000  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.5029  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4667  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.686
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.493
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.457
```

`rrf(k=2,w=1.5:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6169  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6150  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4329  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4298  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.832
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.605
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.603
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.421
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.424
```

`rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6429  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6190  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4653  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4571  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.630
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.607
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.456
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.448
```

`rrf(k=2,w=3:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8667  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6800  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6400  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5143  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5029  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.849
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.666
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.627
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.504
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.493
```

Dense and keyword search (stop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.7594  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7287  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6543  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.6369  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.6338  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
    5. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4500  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3929  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3667  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.441
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.385
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.359
```

`dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9565  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8885  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5337  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5084  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.5037  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.937
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.871
    [DOC #3] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.523
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.498
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.494
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5000  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.2917  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.2500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.2250  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.490
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.286
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.245
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.220
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8333  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6696  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.6071  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5903  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.817
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.656
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.595
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.578
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9091  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8013  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.7500  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.7418  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.891
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.785
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.735
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.727
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9836  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9601  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.9454  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.9449  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.964
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.941
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.927
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.926
```

`rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4848  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4091  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3994  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.475
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.401
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.391
```

`rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5143  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4286  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4286  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.504
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.420
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.420
```

`rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5600  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4762  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4667  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.549
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.467
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.457
```

`rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4752  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4329  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.3916  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.466
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.424
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.384
```

`rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5000  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4653  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4163  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.490
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.456
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.408
```

`rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5429  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.5143  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4600  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.532
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.504
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.451
```

### q40-checkout-outage-root-cause (must retrieve ["incident_inc-checkout-outage"]): "What was the root cause of the last checkout outage?"

Dense and keyword search (nostop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.7136  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.6511  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.6266  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 24.6248  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9091  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8333  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.879
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.688
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9836  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.9677  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.951
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.799
```

Dense and keyword search (stop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.7136  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.6511  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.6266  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 18.8772  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

### q41-gateway-postmortem-actions (must retrieve ["incident_inc-gateway-cert"]): "Which action items came out of the postmortem on failed card payments at the bank gateway?"

Dense and keyword search (nostop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6148  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.6098  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 39.8123  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8333  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.688
```

`dbsf nostop`, `dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.5000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.5000  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.543
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.413
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.7500  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.814
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.619
```

`rrf(k=5) nostop`, `rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.9167  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.995
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.756
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9545  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.036
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.788
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9918  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.077
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.818
```

`rrf(k=2,w=1:1.5) nostop`, `rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8442  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.917
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.700
```

`rrf(k=2,w=1:2) nostop`, `rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8571  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.931
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.707
```

`rrf(k=2,w=1:3) nostop`, `rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8667  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.956
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.715
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8442  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.921
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.696
```

`rrf(k=2,w=2:1) nostop`, `rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8571  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.931
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.707
```

`rrf(k=2,w=3:1) nostop`, `rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8667  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.941
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.726
```

Dense and keyword search (stop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6148  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.6098  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 30.8881  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.688
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.7500  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.814
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.619
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.9918  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.077
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.818
```

### q50-checkout-owner-runbook (must retrieve ["catalog_checkout"]): "Who owns checkout and where is its runbook?"

Dense and keyword search (nostop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6228  catalog_checkout  Service: checkout
    2. 0.5506  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5436  logpattern_a9d39443cdb23ad4fcc19474e198a33f_2026-02-16  Log: checkout - error - checkout redis connection refused: ECONNREFUSED #.#.#.#:# (cart session store)
    4. 0.5430  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-29  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.5426  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-28  Log: checkout - error - checkout tax service returned # for region SE after #ms
  keyword search for "Who owns checkout and where is its runbook?":
    1. 8.6347  monitor_1001  Checkout p95 latency above 1.5s
    2. 8.5864  catalog_checkout  Service: checkout
    3. 5.8509  monitor_1002  Checkout p95 latency above 1.5s (staging)
    4. 4.4920  dashboard_dash-checkout  Checkout overview
    5. 4.0930  incident_inc-checkout-outage  Orders stuck at the payment step
  dense search for "checkout owner team contacts runbook":
    1. 0.6846  catalog_checkout  Service: checkout
    2. 0.5703  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5702  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5669  logpattern_78b9e13a0700a5e105eab795055a15d7_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5624  logpattern_d440cd4c56b8f63b1c70bc320210dfa2_2026-03-09  Log: checkout - warn - slow query on orders table (#ms)
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9959  catalog_checkout  Service: checkout
    2. 0.8947  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.8818  dashboard_dash-checkout  Checkout overview
    4. 0.8474  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-26  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.8460  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-28  Log: checkout - error - checkout tax service returned # for region SE after #ms
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.781
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.747
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.671
    [DOC #4] Orders stuck at the payment step (Incident)  score 0.738
    [DOC #5] Checkout overview (Dashboard)  score 0.661
```

Dense and keyword search (stop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6228  catalog_checkout  Service: checkout
    2. 0.5506  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5436  logpattern_a9d39443cdb23ad4fcc19474e198a33f_2026-02-16  Log: checkout - error - checkout redis connection refused: ECONNREFUSED #.#.#.#:# (cart session store)
    4. 0.5430  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-29  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.5426  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-28  Log: checkout - error - checkout tax service returned # for region SE after #ms
  keyword search for "Who owns checkout and where is its runbook?":
    1. 5.9769  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
  dense search for "checkout owner team contacts runbook":
    1. 0.6846  catalog_checkout  Service: checkout
    2. 0.5703  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.5702  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.5669  logpattern_78b9e13a0700a5e105eab795055a15d7_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.5624  logpattern_d440cd4c56b8f63b1c70bc320210dfa2_2026-03-09  Log: checkout - warn - slow query on orders table (#ms)
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  catalog_checkout  Service: checkout
    2. 0.8896  dashboard_dash-checkout  Checkout overview
    3. 0.8487  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-28  Log: checkout - error - checkout tax service returned # for region SE after #ms
    4. 0.8474  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-26  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.8439  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-29  Log: checkout - error - checkout tax service returned # for region SE after #ms
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.792
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.750
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.673
    [DOC #4] Checkout overview (Dashboard)  score 0.667
    [DOC #5] Orders stuck at the payment step (Incident)  score 0.684
```

## intfloat/e5-small-v2

### q17-inventory-logs-by-service (must retrieve ["logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e"]): "Which errors did the inventory service log yesterday?"

Dense and keyword search (nostop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.8550  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8312  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8120  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    4. 0.8080  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    5. 0.8058  logpattern_3f2446ccfe0adbf7ac1fc228106ba628_2026-03-11  Log: checkout-api - error - PaymentDeclinedException: issuer declined card on retry # of #
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 8.1636  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    2. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    3. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7000  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.5833  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3429  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3056  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.817
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.686
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.572
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.336
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.299
```

`dbsf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9227  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7846  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7391  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5016  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.4701  logpattern_3f2446ccfe0adbf7ac1fc228106ba628_2026-03-11  Log: checkout-api - error - PaymentDeclinedException: issuer declined card on retry # of #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.904
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.769
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.724
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.492
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.460
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6250  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.4167  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.2083  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.1961  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.735
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.613
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.408
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.204
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.192
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8125  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.7738  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.5625  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4762  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.898
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.796
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.758
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.551
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.467
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8846  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.8712  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.7179  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.6250  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.867
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.704
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.608
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9762  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.9757  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.9377  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.9091  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.972
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.956
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.957
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.919
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.885
```

`rrf(k=2,w=1:1.5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8442  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7273  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6061  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3778  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3010  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.827
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.713
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.594
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.370
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.295
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7429  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6286  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4082  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3228  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.728
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.616
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.400
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.316
```

`rrf(k=2,w=1:3) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7600  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4571  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3727  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.745
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.448
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.365
```

`rrf(k=2,w=1.5:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7025  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6169  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3636  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3535  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.832
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.688
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.605
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.356
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.346
```

`rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6429  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3905  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.3857  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.700
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.630
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.383
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.378
```

`rrf(k=2,w=3:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8667  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7429  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6800  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4444  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.4267  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.849
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.728
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.666
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.436
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.418
```

Dense and keyword search (stop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.8550  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8312  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8120  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    4. 0.8080  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    5. 0.8058  logpattern_3f2446ccfe0adbf7ac1fc228106ba628_2026-03-11  Log: checkout-api - error - PaymentDeclinedException: issuer declined card on retry # of #
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.3929  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3056  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.2917  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.385
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.299
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.286
```

`dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9969  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8531  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5086  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    4. 0.4889  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    5. 0.4772  logpattern_3f2446ccfe0adbf7ac1fc228106ba628_2026-03-11  Log: checkout-api - error - PaymentDeclinedException: issuer declined card on retry # of #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.977
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.836
    [DOC #3] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.498
    [DOC #4] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.479
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.467
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5000  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.2500  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.1961  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.1750  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.490
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.245
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.192
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.168
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8333  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6071  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.5051  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.4911  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.817
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.595
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.495
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.481
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9091  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7500  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.6696  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.6478  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.891
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.735
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.656
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.635
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9836  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9454  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.9233  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 0.9110  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.964
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.927
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.905
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.893
```

`rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4329  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3306  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3234  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.424
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.317
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.324
```

`rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4653  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3636  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3520  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.456
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.356
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.345
```

`rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5143  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4156  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4000  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.504
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.407
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.392
```

`rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4091  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3535  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.3237  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.401
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.346
    [DOC #5] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.317
```

`rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4286  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3905  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.3571  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.420
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.383
    [DOC #5] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.350
```

`rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4667  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4444  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.4095  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.457
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.436
    [DOC #5] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.401
```

### q31-exact-metric-name (must retrieve ["monitor_1101"]): "Which storefront monitor uses trace.http.request.errors?"

Dense and keyword search (nostop):

```
  dense search for "Which storefront monitor uses trace.http.request.errors?":
    1. 0.8811  monitor_1102  Storefront traffic monitor
    2. 0.8728  monitor_1103  Storefront latency above 3s
    3. 0.8711  monitor_1101  Storefront 5xx responses
    4. 0.8596  monitor_1104  Storefront Apdex
  keyword search for "Which storefront monitor uses trace.http.request.errors?":
    1. 27.6051  monitor_1101  Storefront 5xx responses
    2. 22.2702  monitor_1103  Storefront latency above 3s
    3. 19.7285  monitor_1102  Storefront traffic monitor
    4. 18.9671  monitor_1104  Storefront Apdex
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7727  monitor_1102  Storefront traffic monitor
    2. 0.7576  monitor_1101  Storefront 5xx responses
    3. 0.6926  monitor_1103  Storefront latency above 3s
    4. 0.4298  monitor_1104  Storefront Apdex
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Storefront traffic monitor (Monitor)  score 0.609
    [DOC #2] Storefront 5xx responses (Monitor)  score 0.597
    [DOC #3] Storefront latency above 3s (Monitor)  score 0.545
    [DOC #4] Storefront Apdex (Monitor)  score 0.338
```

`rrf(k=2,w=2:1) nostop`, `rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7857  monitor_1102  Storefront traffic monitor
    2. 0.7714  monitor_1101  Storefront 5xx responses
    3. 0.7143  monitor_1103  Storefront latency above 3s
    4. 0.4571  monitor_1104  Storefront Apdex
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Storefront traffic monitor (Monitor)  score 0.619
    [DOC #2] Storefront 5xx responses (Monitor)  score 0.607
    [DOC #3] Storefront latency above 3s (Monitor)  score 0.562
    [DOC #4] Storefront Apdex (Monitor)  score 0.360
```

Dense and keyword search (stop):

```
  dense search for "Which storefront monitor uses trace.http.request.errors?":
    1. 0.8811  monitor_1102  Storefront traffic monitor
    2. 0.8728  monitor_1103  Storefront latency above 3s
    3. 0.8711  monitor_1101  Storefront 5xx responses
    4. 0.8596  monitor_1104  Storefront Apdex
  keyword search for "Which storefront monitor uses trace.http.request.errors?":
    1. 27.6051  monitor_1101  Storefront 5xx responses
    2. 22.2702  monitor_1103  Storefront latency above 3s
    3. 19.7285  monitor_1102  Storefront traffic monitor
    4. 18.9671  monitor_1104  Storefront Apdex
```

### q40-checkout-outage-root-cause (must retrieve ["incident_inc-checkout-outage"]): "What was the root cause of the last checkout outage?"

Dense and keyword search (nostop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.8562  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8445  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8422  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 24.6248  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9091  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8333  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.879
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.688
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9836  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.9677  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.951
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.799
```

Dense and keyword search (stop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.8562  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8445  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8422  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 18.8772  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

### q41-gateway-postmortem-actions (must retrieve ["incident_inc-gateway-cert"]): "Which action items came out of the postmortem on failed card payments at the bank gateway?"

Dense and keyword search (nostop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.8226  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8109  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 39.8123  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf nostop`, `rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8333  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.688
```

`dbsf nostop`, `dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.5000  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.5000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.543
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.413
```

`rrf(k=1) nostop`, `rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.7500  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.814
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.619
```

`rrf(k=5) nostop`, `rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9167  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.995
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.756
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9545  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.036
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.788
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.9918  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.077
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.818
```

`rrf(k=2,w=1:1.5) nostop`, `rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8442  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.917
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.700
```

`rrf(k=2,w=1:2) nostop`, `rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8571  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.931
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.707
```

`rrf(k=2,w=1:3) nostop`, `rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8667  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.956
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.715
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8442  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.921
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.696
```

`rrf(k=2,w=2:1) nostop`, `rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8571  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.931
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.707
```

`rrf(k=2,w=3:1) nostop`, `rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8667  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.941
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.726
```

Dense and keyword search (stop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.8226  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8109  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 30.8881  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9918  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.077
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.818
```

### q50-checkout-owner-runbook (must retrieve ["catalog_checkout"]): "Who owns checkout and where is its runbook?"

Dense and keyword search (nostop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.8332  catalog_checkout  Service: checkout
    2. 0.7978  slo_slo-checkout-avail  Checkout availability
    3. 0.7963  change_dep-co-1  Deployed checkout 2.14.0 to prod
    4. 0.7955  dashboard_dash-checkout  Checkout overview
    5. 0.7927  incident_inc-checkout-outage  Orders stuck at the payment step
  keyword search for "Who owns checkout and where is its runbook?":
    1. 8.6347  monitor_1001  Checkout p95 latency above 1.5s
    2. 8.5864  catalog_checkout  Service: checkout
    3. 5.8509  monitor_1002  Checkout p95 latency above 1.5s (staging)
    4. 4.4920  dashboard_dash-checkout  Checkout overview
    5. 4.0930  incident_inc-checkout-outage  Orders stuck at the payment step
  dense search for "checkout owner team contacts runbook":
    1. 0.8651  catalog_checkout  Service: checkout
    2. 0.8256  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-16  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    3. 0.8253  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-15  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    4. 0.8252  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-13  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    5. 0.8252  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-17  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9959  catalog_checkout  Service: checkout
    2. 0.9096  dashboard_dash-checkout  Checkout overview
    3. 0.8602  monitor_1001  Checkout p95 latency above 1.5s
    4. 0.8596  slo_slo-checkout-avail  Checkout availability
    5. 0.8579  incident_inc-checkout-outage  Orders stuck at the payment step
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.778
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.747
    [DOC #3] Checkout overview (Dashboard)  score 0.682
    [DOC #4] Checkout availability (SLO)  score 0.664
    [DOC #5] Orders stuck at the payment step (Incident)  score 0.708
```

Dense and keyword search (stop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.8332  catalog_checkout  Service: checkout
    2. 0.7978  slo_slo-checkout-avail  Checkout availability
    3. 0.7963  change_dep-co-1  Deployed checkout 2.14.0 to prod
    4. 0.7955  dashboard_dash-checkout  Checkout overview
    5. 0.7927  incident_inc-checkout-outage  Orders stuck at the payment step
  keyword search for "Who owns checkout and where is its runbook?":
    1. 5.9769  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
  dense search for "checkout owner team contacts runbook":
    1. 0.8651  catalog_checkout  Service: checkout
    2. 0.8256  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-16  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    3. 0.8253  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-15  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    4. 0.8252  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-13  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    5. 0.8252  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-17  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  catalog_checkout  Service: checkout
    2. 0.9174  dashboard_dash-checkout  Checkout overview
    3. 0.8661  slo_slo-checkout-avail  Checkout availability
    4. 0.8435  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-16  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
    5. 0.8432  slo_slo-checkout-avail-stg  Checkout availability (staging)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.788
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.750
    [DOC #3] Checkout overview (Dashboard)  score 0.688
    [DOC #4] Checkout availability (SLO)  score 0.669
    [DOC #5] Deployed checkout 2.14.0 to prod (Change)  score 0.637
```

## intfloat/e5-base-v2

### q17-inventory-logs-by-service (must retrieve ["logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e"]): "Which errors did the inventory service log yesterday?"

Dense and keyword search (nostop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.8242  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8075  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7805  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    4. 0.7803  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.7792  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 8.1636  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    2. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    3. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.5833  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3429  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3111  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.817
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.653
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.572
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.336
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.305
```

`dbsf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8889  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8034  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7605  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5081  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5079  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.871
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.787
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.745
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.498
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.498
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6000  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.4167  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.2083  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.1961  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.735
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.588
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.408
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.204
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.192
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7778  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.7738  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.5625  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.5208  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.898
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.758
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.762
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.551
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.510
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8712  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8571  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.7179  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.6787  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.840
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.704
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.665
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9757  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9687  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.9377  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.9240  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.972
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.956
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.949
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.919
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.905
```

`rrf(k=2,w=1:1.5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8442  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6970  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6061  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3778  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3254  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.827
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.683
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.594
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.370
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.319
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6286  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4082  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3429  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.700
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.616
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.400
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.336
```

`rrf(k=2,w=1:3) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7333  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4571  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3800  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.719
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.448
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.372
```

`rrf(k=2,w=1.5:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6643  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6169  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3636  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3535  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.832
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.651
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.605
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.356
    [DOC #5] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.346
```

`rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6735  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6429  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3905  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.3857  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.660
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.630
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.383
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.378
```

`rrf(k=2,w=3:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8667  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7000  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6800  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4444  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.4317  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.849
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.686
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.666
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.436
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.423
```

Dense and keyword search (stop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.8242  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8075  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7805  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    4. 0.7803  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.7792  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    5. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.3929  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3250  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3056  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.385
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.299
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.318
```

`dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9630  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8719  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5153  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    4. 0.5149  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.5104  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.944
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #3] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.505
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.505
    [DOC #5] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.500
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5000  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.2500  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.1964  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.1961  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.490
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.245
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.192
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.192
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8333  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6071  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.5398  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5051  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.817
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.595
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.529
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.495
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9091  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7500  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.6971  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.6696  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.891
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.735
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.683
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.656
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9836  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9454  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.9307  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.9233  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.964
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.927
    [DOC #4] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.905
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.912
```

`rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4329  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3422  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3388  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.424
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.332
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.317
```

`rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4653  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3714  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3619  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.456
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.364
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.345
```

`rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5143  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4229  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4000  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.504
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.414
    [DOC #5] Log: checkout - error - db connection pool exhausted: #/# connections in use (Logs)  score 0.392
```

`rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4091  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3616  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3535  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.401
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.346
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.354
```

`rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4286  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.3929  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3905  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.420
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.383
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.385
```

`rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4667  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 0.4444  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
    5. 0.4429  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.457
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.436
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.434
```

### q25-failing-right-now (must retrieve ["logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763"]): "What is failing in notifications right now?"

Dense and keyword search (nostop):

```
  dense search for "What is failing in notifications right now?":
    1. 0.8496  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.8366  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "What is failing in notifications right now?":
    1. 22.4156  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 14.6065  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  dense search for "notifications delivery failing errors":
    1. 0.8692  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.8651  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "notifications delivery failing errors":
    1. 21.6545  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
    2. 21.4897  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
```

`rrf(k=1) nostop`, `rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8750  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.6250  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired (Logs)  score 0.650
    [DOC #2] Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay (Logs)  score 0.611
```

Dense and keyword search (stop):

```
  dense search for "What is failing in notifications right now?":
    1. 0.8496  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.8366  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "What is failing in notifications right now?":
    1. 22.4156  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 14.6065  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  dense search for "notifications delivery failing errors":
    1. 0.8692  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
    2. 0.8651  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
  keyword search for "notifications delivery failing errors":
    1. 21.6545  logpattern_4cefdd4dea36d4c5991e2cc9c6ce3763_2026-03-12  Log: notifications - error - notifications failing: email delivery rejected by the SMTP relay
    2. 21.4897  logpattern_4bf138fdadd8c61de9cdd7e162c6b0a1_2026-03-07  Log: notifications - error - notifications failing now: push delivery rejected by APNs, key expired
```

### q40-checkout-outage-root-cause (must retrieve ["incident_inc-checkout-outage"]): "What was the root cause of the last checkout outage?"

Dense and keyword search (nostop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.8346  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8111  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8032  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 24.6248  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9091  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8333  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.879
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.688
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9836  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.9677  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.951
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.799
```

Dense and keyword search (stop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.8346  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8111  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.8032  incident_inc-checkout-deploy  Checkout errors after deploy
  keyword search for "What was the root cause of the last checkout outage?":
    1. 18.8772  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

### q41-gateway-postmortem-actions (must retrieve ["incident_inc-gateway-cert"]): "Which action items came out of the postmortem on failed card payments at the bank gateway?"

Dense and keyword search (nostop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.7846  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.7805  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 39.8123  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf nostop`, `rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8333  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.688
```

`dbsf nostop`, `dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.5000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.5000  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.543
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.413
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.7500  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.814
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.619
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9167  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.995
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.756
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.9545  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.036
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.788
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9918  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.077
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.818
```

`rrf(k=2,w=1:1.5) nostop`, `rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8442  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.917
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.700
```

`rrf(k=2,w=1:2) nostop`, `rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8571  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.931
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.707
```

`rrf(k=2,w=1:3) nostop`, `rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8667  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.956
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.715
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8442  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.921
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.696
```

`rrf(k=2,w=3:1) nostop`, `rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8667  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.941
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.726
```

Dense and keyword search (stop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.7846  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.7805  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 30.8881  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.7500  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.814
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.619
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.9167  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.995
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.756
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9545  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.036
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.788
```

`rrf(k=2,w=1:2) stop`, `rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
    2. 0.8571  incident_inc-gateway-cert  TLS handshake errors to the acquirer
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.931
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.707
```

### q50-checkout-owner-runbook (must retrieve ["catalog_checkout"]): "Who owns checkout and where is its runbook?"

Dense and keyword search (nostop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.8276  catalog_checkout  Service: checkout
    2. 0.7846  slo_slo-checkout-avail  Checkout availability
    3. 0.7837  logpattern_a9d39443cdb23ad4fcc19474e198a33f_2026-02-20  Log: checkout - error - checkout redis connection refused: ECONNREFUSED #.#.#.#:# (cart session store)
    4. 0.7835  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.7793  change_dep-co-1  Deployed checkout 2.14.0 to prod
  keyword search for "Who owns checkout and where is its runbook?":
    1. 8.6347  monitor_1001  Checkout p95 latency above 1.5s
    2. 8.5864  catalog_checkout  Service: checkout
    3. 5.8509  monitor_1002  Checkout p95 latency above 1.5s (staging)
    4. 4.4920  dashboard_dash-checkout  Checkout overview
    5. 4.0930  incident_inc-checkout-outage  Orders stuck at the payment step
  dense search for "checkout owner team contacts runbook":
    1. 0.8438  catalog_checkout  Service: checkout
    2. 0.7928  logpattern_a9d39443cdb23ad4fcc19474e198a33f_2026-02-20  Log: checkout - error - checkout redis connection refused: ECONNREFUSED #.#.#.#:# (cart session store)
    3. 0.7913  logpattern_78b9e13a0700a5e105eab795055a15d7_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.7902  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.7882  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9959  catalog_checkout  Service: checkout
    2. 0.8845  slo_slo-checkout-avail  Checkout availability
    3. 0.8791  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.8651  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-26  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.8648  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-28  Log: checkout - error - checkout tax service returned # for region SE after #ms
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.785
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.747
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.659
    [DOC #4] Checkout availability (SLO)  score 0.683
    [DOC #5] Orders stuck at the payment step (Incident)  score 0.725
```

Dense and keyword search (stop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.8276  catalog_checkout  Service: checkout
    2. 0.7846  slo_slo-checkout-avail  Checkout availability
    3. 0.7837  logpattern_a9d39443cdb23ad4fcc19474e198a33f_2026-02-20  Log: checkout - error - checkout redis connection refused: ECONNREFUSED #.#.#.#:# (cart session store)
    4. 0.7835  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.7793  change_dep-co-1  Deployed checkout 2.14.0 to prod
  keyword search for "Who owns checkout and where is its runbook?":
    1. 5.9769  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
  dense search for "checkout owner team contacts runbook":
    1. 0.8438  catalog_checkout  Service: checkout
    2. 0.7928  logpattern_a9d39443cdb23ad4fcc19474e198a33f_2026-02-20  Log: checkout - error - checkout redis connection refused: ECONNREFUSED #.#.#.#:# (cart session store)
    3. 0.7913  logpattern_78b9e13a0700a5e105eab795055a15d7_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
    4. 0.7902  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.7882  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  catalog_checkout  Service: checkout
    2. 0.8910  slo_slo-checkout-avail  Checkout availability
    3. 0.8737  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-26  Log: checkout - error - checkout tax service returned # for region SE after #ms
    4. 0.8648  logpattern_4f5ed8ced7fe9eda9a6792be4bb8672c_2026-01-28  Log: checkout - error - checkout tax service returned # for region SE after #ms
    5. 0.8604  slo_slo-checkout-avail-stg  Checkout availability (staging)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.796
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.750
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.661
    [DOC #4] Checkout availability (SLO)  score 0.688
    [DOC #5] Deployed checkout 2.13.2 to prod (Change)  score 0.642
```

## nomic-ai/nomic-embed-text-v1.5

### q17-inventory-logs-by-service (must retrieve ["logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e"]): "Which errors did the inventory service log yesterday?"

Dense and keyword search (nostop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.7869  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7720  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7243  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    4. 0.7228  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.7113  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 8.1636  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    2. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    3. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
```

`rrf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8333  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6429  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.5833  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3667  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3611  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.817
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.630
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.572
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.359
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.354
```

`dbsf nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8594  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8117  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7507  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5295  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5266  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.842
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.795
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.736
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.516
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.519
```

`rrf(k=1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7500  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5833  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.4167  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.2292  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.2250  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.735
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.572
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.408
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.220
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.225
```

`rrf(k=5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9167  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7738  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7500  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.5903  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.5655  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.898
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.758
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.735
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.578
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.554
```

`rrf(k=10) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9545  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8712  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.8333  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.7418  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.7108  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.935
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.854
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.817
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.727
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.697
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9918  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9757  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9615  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.9449  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.9316  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.972
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.956
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.942
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.926
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.905
```

`rrf(k=2,w=1:1.5) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8442  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6753  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6061  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.3916  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3708  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.827
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.662
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.594
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.384
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.342
```

`rrf(k=2,w=1:2) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6939  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6286  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4163  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.3857  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.680
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.616
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.408
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.373
```

`rrf(k=2,w=1:3) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8800  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4600  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.4317  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.862
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.700
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.451
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.423
```

`rrf(k=2,w=1.5:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8485  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6364  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    3. 0.6169  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    4. 0.4040  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3994  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.832
    [DOC #2] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.624
    [DOC #3] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.605
    [DOC #4] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.391
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.396
```

`rrf(k=2,w=2:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8571  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6429  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6429  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4381  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.4286  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.840
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.630
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.630
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.429
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.420
```

`rrf(k=2,w=3:1) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.8667  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6800  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6667  logpattern_d5bbf0bcb64210563af9ebfd3d0a4581_2026-03-11  Log: checkout-api - error - retries exhausted: card vault did not respond within #s
    4. 0.4889  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.4762  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.849
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.666
    [DOC #3] Log: checkout-api - error - retries exhausted: card vault did not respond within #s (Logs)  score 0.653
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.479
    [DOC #5] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.467
```

Dense and keyword search (stop):

```
  dense search for "Which errors did the inventory service log yesterday?":
    1. 0.7869  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7720  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7243  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    4. 0.7228  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.7113  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  keyword search for "Which errors did the inventory service log yesterday?":
    1. 6.7818  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 6.3545  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 1.7718  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    4. 1.7669  logpattern_d09d6b3635999e2b527755e4139a8100_2026-03-11  Log: sökmotor - warn - indexering långsam för 'smörgåsbord' – #.# s per sida
    5. 1.7669  logpattern_996323a777e3701ecfc644eba0e31a63_2026-03-11  Log: checkout - error - db connection pool exhausted: #/# connections in use
```

`rrf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6667  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4000  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.3750  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3611  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.653
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.392
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.354
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.368
```

`dbsf stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9335  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8802  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5367  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    4. 0.5340  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    5. 0.5091  logpattern_aff7335bb8a8c65e31d893cd75e91d81_2026-03-11  Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.915
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.863
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.523
    [DOC #4] Log: checkout-api - error - #-D Secure challenge failed for card, retries disabled for this merchant (Logs)  score 0.499
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.526
```

`rrf(k=1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.5000  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.2500  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.2381  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.2292  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.490
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.245
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.225
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.233
```

`rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.8333  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.6250  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.5844  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.5655  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.817
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.613
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.554
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.573
```

`rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9091  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.7692  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.7292  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.7108  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.891
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.754
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.697
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.715
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.9836  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.9524  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.9384  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.9316  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.964
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.933
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.913
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.920
```

`rrf(k=2,w=1:1.5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4298  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.4040  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.3877  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.421
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.396
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.380
```

`rrf(k=2,w=1:2) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4571  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.4381  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4048  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.448
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.429
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.397
```

`rrf(k=2,w=1:3) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5029  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.4889  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
    5. 0.4400  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.493
    [DOC #4] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.479
    [DOC #5] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.431
```

`rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.6926  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4298  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.4167  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3708  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.679
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.421
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.408
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.363
```

`rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7143  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.4571  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.4500  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.3857  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.700
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.448
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.441
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.378
```

`rrf(k=2,w=3:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  logpattern_12a088b81c8ca50729a64fc94640b0e1_2026-03-11  Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending
    2. 0.7467  logpattern_0ab0d0bcbc0d4bdde36fff39ba11da5e_2026-03-11  Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde…
    3. 0.5029  logpattern_6cd04acdaeaad52e1f5a4751ee3450a2_2026-03-11  Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments)
    4. 0.5000  logpattern_2ef4aec60f5cc7c61350da0876a71fc3_2026-03-11  Log: checkout-api - error - throw away stale cart session after checkout timeout
    5. 0.4182  logpattern_e036dc5062c01f94cbb281bfb3a0d82f_2026-03-11  Log: checkout-worker - warn - order queue depth # exceeds limit #
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Log: orders - error - inventory client error: POST /reservations returned #, order # kept pending (Logs)  score 0.980
    [DOC #2] Log: inventory - error - stock reservation failed: lock wait timeout exceeded on table reservations (orde… (Logs)  score 0.732
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.493
    [DOC #4] Log: checkout-api - error - throw away stale cart session after checkout timeout (Logs)  score 0.490
    [DOC #5] Log: checkout-worker - warn - order queue depth # exceeds limit # (Logs)  score 0.410
```

### q31-exact-metric-name (must retrieve ["monitor_1101"]): "Which storefront monitor uses trace.http.request.errors?"

Dense and keyword search (nostop):

```
  dense search for "Which storefront monitor uses trace.http.request.errors?":
    1. 0.7709  monitor_1102  Storefront traffic monitor
    2. 0.7589  monitor_1103  Storefront latency above 3s
    3. 0.7588  monitor_1101  Storefront 5xx responses
    4. 0.7448  monitor_1104  Storefront Apdex
  keyword search for "Which storefront monitor uses trace.http.request.errors?":
    1. 27.6051  monitor_1101  Storefront 5xx responses
    2. 22.2702  monitor_1103  Storefront latency above 3s
    3. 19.7285  monitor_1102  Storefront traffic monitor
    4. 18.9671  monitor_1104  Storefront Apdex
```

`rrf(k=2,w=1.5:1) nostop`, `rrf(k=2,w=1.5:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7727  monitor_1102  Storefront traffic monitor
    2. 0.7576  monitor_1101  Storefront 5xx responses
    3. 0.6926  monitor_1103  Storefront latency above 3s
    4. 0.4298  monitor_1104  Storefront Apdex
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Storefront traffic monitor (Monitor)  score 0.609
    [DOC #2] Storefront 5xx responses (Monitor)  score 0.597
    [DOC #3] Storefront latency above 3s (Monitor)  score 0.545
    [DOC #4] Storefront Apdex (Monitor)  score 0.338
```

`rrf(k=2,w=2:1) nostop`, `rrf(k=2,w=2:1) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.7857  monitor_1102  Storefront traffic monitor
    2. 0.7714  monitor_1101  Storefront 5xx responses
    3. 0.7143  monitor_1103  Storefront latency above 3s
    4. 0.4571  monitor_1104  Storefront Apdex
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Storefront traffic monitor (Monitor)  score 0.619
    [DOC #2] Storefront 5xx responses (Monitor)  score 0.607
    [DOC #3] Storefront latency above 3s (Monitor)  score 0.562
    [DOC #4] Storefront Apdex (Monitor)  score 0.360
```

Dense and keyword search (stop):

```
  dense search for "Which storefront monitor uses trace.http.request.errors?":
    1. 0.7709  monitor_1102  Storefront traffic monitor
    2. 0.7589  monitor_1103  Storefront latency above 3s
    3. 0.7588  monitor_1101  Storefront 5xx responses
    4. 0.7448  monitor_1104  Storefront Apdex
  keyword search for "Which storefront monitor uses trace.http.request.errors?":
    1. 27.6051  monitor_1101  Storefront 5xx responses
    2. 22.2702  monitor_1103  Storefront latency above 3s
    3. 19.7285  monitor_1102  Storefront traffic monitor
    4. 18.9671  monitor_1104  Storefront Apdex
```

### q40-checkout-outage-root-cause (must retrieve ["incident_inc-checkout-outage"]): "What was the root cause of the last checkout outage?"

Dense and keyword search (nostop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.7740  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.7369  incident_inc-checkout-deploy  Checkout errors after deploy
    3. 0.7264  incident_inc-checkout-latency  Checkout latency degradation
  keyword search for "What was the root cause of the last checkout outage?":
    1. 24.6248  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.8712  incident_inc-checkout-deploy  Checkout errors after deploy
    3. 0.8712  incident_inc-checkout-latency  Checkout latency degradation
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.842
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.719
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.9757  incident_inc-checkout-latency  Checkout latency degradation
    3. 0.9757  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.943
    [DOC #2] Orders stuck at the payment step (Incident)  score 0.825
    [DOC #3] Checkout errors after deploy (Incident)  score 0.805
```

Dense and keyword search (stop):

```
  dense search for "What was the root cause of the last checkout outage?":
    1. 0.7740  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 0.7369  incident_inc-checkout-deploy  Checkout errors after deploy
    3. 0.7264  incident_inc-checkout-latency  Checkout latency degradation
  keyword search for "What was the root cause of the last checkout outage?":
    1. 18.8772  incident_inc-checkout-outage  Orders stuck at the payment step
    2. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    3. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

### q41-gateway-postmortem-actions (must retrieve ["incident_inc-gateway-cert"]): "Which action items came out of the postmortem on failed card payments at the bank gateway?"

Dense and keyword search (nostop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6553  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.6458  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 39.8123  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

`rrf(k=5) nostop`, `rrf(k=5) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.8333  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.905
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

`rrf(k=10) nostop`, `rrf(k=10) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9091  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 0.987
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

`rrf(k=60) nostop`, `rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.9836  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Kortbetalningar misslyckas 💳 (決済エラー) (Incident)  score 1.068
    [DOC #2] TLS handshake errors to the acquirer (Incident)  score 0.825
```

Dense and keyword search (stop):

```
  dense search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 0.6553  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 0.6458  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
  keyword search for "Which action items came out of the postmortem on failed card payments at the bank gateway?":
    1. 30.8881  incident_inc-gateway-cert  TLS handshake errors to the acquirer
    2. 17.0205  incident_inc-payments-card  Kortbetalningar misslyckas 💳 (決済エラー)
```

### q50-checkout-owner-runbook (must retrieve ["catalog_checkout"]): "Who owns checkout and where is its runbook?"

Dense and keyword search (nostop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6557  catalog_checkout  Service: checkout
    2. 0.6060  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.6031  slo_slo-checkout-avail  Checkout availability
    4. 0.5966  slo_slo-checkout-avail-stg  Checkout availability (staging)
    5. 0.5870  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-14  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
  keyword search for "Who owns checkout and where is its runbook?":
    1. 8.6347  monitor_1001  Checkout p95 latency above 1.5s
    2. 8.5864  catalog_checkout  Service: checkout
    3. 5.8509  monitor_1002  Checkout p95 latency above 1.5s (staging)
    4. 4.4920  dashboard_dash-checkout  Checkout overview
    5. 4.0930  incident_inc-checkout-outage  Orders stuck at the payment step
  dense search for "checkout owner team contacts runbook":
    1. 0.6775  catalog_checkout  Service: checkout
    2. 0.6155  slo_slo-checkout-avail  Checkout availability
    3. 0.6152  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.6039  slo_slo-checkout-avail-stg  Checkout availability (staging)
    5. 0.6010  change_dep-co-1  Deployed checkout 2.14.0 to prod
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) nostop`:

```
  hybrid search (64 candidates, normalized score):
    1. 0.9959  catalog_checkout  Service: checkout
    2. 0.9291  slo_slo-checkout-avail  Checkout availability
    3. 0.9241  slo_slo-checkout-avail-stg  Checkout availability (staging)
    4. 0.8908  incident_inc-checkout-outage  Orders stuck at the payment step
    5. 0.8851  dashboard_dash-checkout  Checkout overview
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.807
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.747
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.694
    [DOC #4] Checkout availability (SLO)  score 0.718
    [DOC #5] Orders stuck at the payment step (Incident)  score 0.735
```

Dense and keyword search (stop):

```
  dense search for "Who owns checkout and where is its runbook?":
    1. 0.6557  catalog_checkout  Service: checkout
    2. 0.6060  incident_inc-checkout-outage  Orders stuck at the payment step
    3. 0.6031  slo_slo-checkout-avail  Checkout availability
    4. 0.5966  slo_slo-checkout-avail-stg  Checkout availability (staging)
    5. 0.5870  logpattern_45d02a23cccc59fefa9515a94cd813b4_2026-01-14  Log: checkout - error - checkout cache timeout: redis GET took #ms for key cart:<uuid>
  keyword search for "Who owns checkout and where is its runbook?":
    1. 5.9769  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
  dense search for "checkout owner team contacts runbook":
    1. 0.6775  catalog_checkout  Service: checkout
    2. 0.6155  slo_slo-checkout-avail  Checkout availability
    3. 0.6152  incident_inc-checkout-outage  Orders stuck at the payment step
    4. 0.6039  slo_slo-checkout-avail-stg  Checkout availability (staging)
    5. 0.6010  change_dep-co-1  Deployed checkout 2.14.0 to prod
  keyword search for "checkout owner team contacts runbook":
    1. 17.2305  catalog_checkout  Service: checkout
    2. 1.8489  dashboard_dash-checkout  Checkout overview
    3. 1.7850  incident_inc-checkout-latency  Checkout latency degradation
    4. 1.7838  incident_inc-checkout-latency-stg  Checkout latency degradation in staging
    5. 1.7815  incident_inc-checkout-deploy  Checkout errors after deploy
```

`rrf(k=60) stop`:

```
  hybrid search (64 candidates, normalized score):
    1. 1.0000  catalog_checkout  Service: checkout
    2. 0.9356  slo_slo-checkout-avail  Checkout availability
    3. 0.9307  slo_slo-checkout-avail-stg  Checkout availability (staging)
    4. 0.8929  dashboard_dash-checkout  Checkout overview
    5. 0.8795  incident_inc-checkout-deploy  Checkout errors after deploy
  reranked sources (score after kind prior and recency weight):
    [DOC #1] Checkout latency degradation (Incident)  score 0.818
    [DOC #2] Service: checkout (ServiceCatalog)  score 0.750
    [DOC #3] Log: checkout - error - upstream timeout calling payments after #ms (checkout → payments) (Logs)  score 0.696
    [DOC #4] Checkout availability (SLO)  score 0.723
    [DOC #5] Checkout overview (Dashboard)  score 0.670
```

