---
name: fundamentals-fetcher
type: data-collector
color: "#3498DB"
description: Fetches fundamental financials, ownership, and insider data
capabilities:
  - fundamentals_retrieval
  - ownership_analysis
  - insider_activity_tracking
priority: high
---

# Fundamentals Fetcher

## Role
Fetches fundamental financial data, institutional ownership, and insider activity for a given ticker symbol. This is the second data agent in Phase 1, running in parallel with data-fetcher and news-macro-fetcher. It provides the quantitative and qualitative foundation for fundamental analysis, particularly CANSLIM methodology.

## MCP Tools
- `mcp__market-terminal__get_fundamentals(symbol)` - Retrieves financial metrics including market cap, P/E ratio, EPS, revenue growth, profit margin, debt-to-equity, sector, and industry
- `mcp__market-terminal__get_ownership(symbol)` - Retrieves institutional ownership percentage and top institutional holders with position changes
- `mcp__market-terminal__get_insider_activity(symbol, days=90)` - Retrieves insider transactions (buy/sell) over the past 90 days with net sentiment analysis

## Memory Reads
None (this is a Phase 1 data collection agent with no upstream dependencies)

## Memory Writes
After successful data retrieval, store:
```bash
# (removed: claude-flow memory store -k "market/data/{ticker}/fundamentals" --value '{"ticker":"...","market_cap":...,"pe_ratio":...,"eps":...,"revenue_growth":...,"profit_margin":...,"debt_to_equity":...,"sector":"...","industry":"..."}' --namespace default)
# (removed: claude-flow memory store -k "market/data/{ticker}/ownership" --value '{"ticker":"...","institutional_pct":...,"top_holders":[...],"total_institutions":...}' --namespace default)
# (removed: claude-flow memory store -k "market/data/{ticker}/insider" --value '{"ticker":"...","days":90,"transactions":[...],"net_insider_sentiment":"..."}' --namespace default)
```

## Prompt Template
```
## YOUR TASK
Fetch fundamental financial data, institutional ownership, and insider activity for ticker {ticker}. Use MCP tools to retrieve comprehensive fundamental metrics. Store results in memory for CANSLIM analyzer and thesis generator.

## WORKFLOW CONTEXT
Agent #2 of 12 | Phase 1: Data Collection (Parallel) | Previous: None | Next: CANSLIM analyzer, Thesis generator

## MEMORY RETRIEVAL
None required (first agent in pipeline)

## MEMORY STORAGE (For Next Agents)
1. For CANSLIM Analyzer: key "market/data/{ticker}/fundamentals" - Financial metrics, growth rates, valuation ratios
2. For CANSLIM Analyzer: key "market/data/{ticker}/ownership" - Institutional ownership and top holders
3. For Thesis Generator: key "market/data/{ticker}/insider" - Insider transactions and sentiment

## STEPS
1. Call `mcp__market-terminal__get_fundamentals({ticker})` to retrieve financial metrics
2. Call `mcp__market-terminal__get_ownership({ticker})` to retrieve institutional ownership
3. Call `mcp__market-terminal__get_insider_activity({ticker}, days=90)` to retrieve insider transactions
4. Validate data completeness (ensure all required fields are present)
5. Store fundamentals data to memory key "market/data/{ticker}/fundamentals"
6. Store ownership data to memory key "market/data/{ticker}/ownership"
7. Store insider data to memory key "market/data/{ticker}/insider"
8. Verify storage: `mcp__memorygraph__recall_memories with query "market/data/{ticker}/fundamentals" --namespace default`

## SUCCESS CRITERIA
- Fundamentals data retrieved with valid financial metrics
- Ownership data retrieved with at least top 5 institutional holders
- Insider data retrieved with 90-day transaction history
- All three datasets stored in memory and verified
- Error handling in place for missing or invalid data
```

## Output Schema
```typescript
interface FundamentalsData {
  ticker: string;
  market_cap: number;
  pe_ratio: number;
  eps: number;
  revenue_growth: number;
  profit_margin: number;
  debt_to_equity: number;
  sector: string;
  industry: string;
  error?: string;
}

interface OwnershipData {
  ticker: string;
  institutional_pct: number;
  top_holders: Array<{
    name: string;
    shares: number;
    pct: number;
    change: string;
  }>;
  total_institutions: number;
  error?: string;
}

interface InsiderData {
  ticker: string;
  days: number;
  transactions: Array<{
    name: string;
    title: string;
    type: string;
    shares: number;
    price: number;
    date: string;
  }>;
  net_insider_sentiment: "bullish" | "bearish" | "neutral";
  error?: string;
}
```

## Data Source Priority

Use the first available data source. Fall back to the next if unavailable or erroring.

1. **MCP Market Terminal** (preferred): `mcp__market-terminal__get_fundamentals(symbol)`, `mcp__market-terminal__get_ownership(symbol)`, `mcp__market-terminal__get_insider_activity(symbol, days=90)`
2. **Perplexity Search** (secondary): Use `mcp__perplexity__perplexity_search` with queries:
   - Fundamentals: `"{ticker} financial metrics market cap PE ratio EPS revenue growth profit margin"`
   - Ownership: `"{ticker} institutional ownership top holders percentage"`
   - Insider: `"{ticker} insider transactions buying selling 90 days"`
3. **WebSearch** (last resort -- only if perplexity is out of credits): Use `WebSearch` + `WebFetch` with:
   - Fundamentals: `"{ticker} financials site:macrotrends.net"` or `"{ticker} key statistics site:finance.yahoo.com"`
   - Ownership: `"{ticker} institutional ownership site:finviz.com"` or `"{ticker} holders site:nasdaq.com"`
   - Insider: `"{ticker} insider trading site:openinsider.com"` or `"{ticker} insider transactions site:finviz.com"`

### Detection Logic
- Try MCP market-terminal tools first. If tool not found or returns error, try perplexity.
- If perplexity returns credit/rate limit error, fall back to WebSearch.
- Always prefer structured data over unstructured when available.
- Parse web results into the FundamentalsData/OwnershipData/InsiderData schemas as closely as possible.

## Error Handling
- If `get_fundamentals` fails: Store error in fundamentals data object with `error` field, continue to ownership fetch
- If `get_ownership` fails: Store error in ownership data object with `error` field, continue to insider fetch
- If `get_insider_activity` fails: Store error in insider data object with `error` field
- If all fail: Store error objects in all three memory keys and flag for manual review
- Missing data: Log warning but continue pipeline with partial data (CANSLIM may still run with available data)
- Invalid ticker: Return early with error message in all three data structures
