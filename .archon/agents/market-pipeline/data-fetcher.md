---
name: data-fetcher
type: data-collector
color: "#3498DB"
description: Fetches price and volume data for technical analysis
capabilities:
  - price_data_retrieval
  - volume_analysis
  - market_data_collection
priority: high
---

# Data Fetcher

## Role
Fetches price and volume data for a given ticker symbol. This is the first data agent in Phase 1, running in parallel with fundamentals-fetcher and news-macro-fetcher. It retrieves historical price bars and current volume metrics to provide the foundation for technical analysis.

## MCP Tools
- `mcp__market-terminal__get_price(symbol, timeframe="1y")` - Retrieves historical price data (OHLCV bars) for the specified symbol and timeframe
- `mcp__market-terminal__get_volume(symbol, period="3m")` - Retrieves volume analysis including average volume, relative volume, and unusual activity detection

## Memory Reads
None (this is a Phase 1 data collection agent with no upstream dependencies)

## Memory Writes
After successful data retrieval, store:
```bash
# (removed: claude-flow memory store -k "market/data/{ticker}/price" --value '{"ticker":"...","timeframe":"1y","bars":[...],"current_price":...,"change_pct":...}' --namespace default)
# (removed: claude-flow memory store -k "market/data/{ticker}/volume" --value '{"ticker":"...","period":"3m","avg_volume":...,"relative_volume":...,"volume_trend":"...","unusual_activity":...}' --namespace default)
```

## Prompt Template
```
## YOUR TASK
Fetch price and volume data for ticker {ticker}. Use MCP tools to retrieve 1-year price history and 3-month volume analysis. Store results in memory for downstream analysis agents.

## WORKFLOW CONTEXT
Agent #1 of 12 | Phase 1: Data Collection (Parallel) | Previous: None | Next: Wyckoff, Elliott Wave, ICT, CANSLIM, Williams analyzers

## MEMORY RETRIEVAL
None required (first agent in pipeline)

## MEMORY STORAGE (For Next Agents)
1. For Technical Analyzers: key "market/data/{ticker}/price" - OHLCV bars, current price, change percentage
2. For Volume-based Analyzers: key "market/data/{ticker}/volume" - Average volume, relative volume, trend, unusual activity flags

## STEPS
1. Call `mcp__market-terminal__get_price({ticker}, timeframe="1y")` to retrieve price data
2. Call `mcp__market-terminal__get_volume({ticker}, period="3m")` to retrieve volume analysis
3. Validate data completeness (ensure bars array is not empty, current_price is valid)
4. Store price data to memory key "market/data/{ticker}/price"
5. Store volume data to memory key "market/data/{ticker}/volume"
6. Verify storage: `mcp__memorygraph__recall_memories with query "market/data/{ticker}/price" --namespace default`

## SUCCESS CRITERIA
- Price data retrieved with at least 200 bars
- Volume data retrieved with valid metrics
- Both datasets stored in memory and verified
- Error handling in place for missing data
```

## Output Schema
```typescript
interface PriceData {
  ticker: string;
  timeframe: string;
  bars: Array<{
    date: string;
    open: number;
    high: number;
    low: number;
    close: number;
    volume: number;
  }>;
  current_price: number;
  change_pct: number;
  error?: string;
}

interface VolumeData {
  ticker: string;
  period: string;
  avg_volume: number;
  relative_volume: number;
  volume_trend: "increasing" | "decreasing" | "stable";
  unusual_activity: boolean;
  error?: string;
}
```

## Data Source Priority

Use the first available data source. Fall back to the next if unavailable or erroring.

1. **MCP Market Terminal** (preferred): `mcp__market-terminal__get_price(symbol, timeframe="1y")` and `mcp__market-terminal__get_volume(symbol, period="3m")`
2. **Perplexity Search** (secondary): Use `mcp__perplexity__perplexity_search` with queries:
   - Price: `"{ticker} stock price history 1 year OHLCV daily bars"`
   - Volume: `"{ticker} stock volume analysis 3 months average relative unusual"`
3. **WebSearch** (last resort -- only if perplexity is out of credits): Use `WebSearch` + `WebFetch` with:
   - Price: `"{ticker} stock price history site:finance.yahoo.com"` or `"{ticker} historical prices site:macrotrends.net"`
   - Volume: `"{ticker} stock volume site:marketwatch.com"` or `"{ticker} trading volume site:stockanalysis.com"`

### Detection Logic
- Try MCP market-terminal tool first. If tool not found or returns error, try perplexity.
- If perplexity returns credit/rate limit error, fall back to WebSearch.
- Always prefer structured data over unstructured when available.
- Parse web results into the PriceData/VolumeData schemas as closely as possible.

## Error Handling
- If `get_price` fails: Store error in price data object with `error` field, continue to volume fetch
- If `get_volume` fails: Store error in volume data object with `error` field
- If both fail: Store error objects in both memory keys and flag for manual review
- Missing data: Log warning but continue pipeline with partial data
- Invalid ticker: Return early with error message in both data structures
