---
name: news-macro-fetcher
type: data-collector
color: "#3498DB"
description: Fetches news, macro calendar, and macro indicators
capabilities:
  - news_retrieval
  - macro_calendar_tracking
  - macro_indicator_analysis
priority: high
---

# News Macro Fetcher

## Role
Fetches news articles, macroeconomic calendar events, and macro indicator history for a given ticker symbol. This is the third data agent in Phase 1, running in parallel with data-fetcher and fundamentals-fetcher. It provides sentiment data and macroeconomic context for sentiment analysis and overall market environment assessment.

## MCP Tools
- `mcp__market-terminal__get_news(symbol, limit=20)` - Retrieves recent news articles with sentiment scores and summaries
- `mcp__market-terminal__get_macro_calendar(days=30)` - Retrieves upcoming macroeconomic events (FOMC, jobs report, GDP, etc.) with impact levels
- `mcp__market-terminal__get_macro_history(indicator, symbol="SPY")` - Retrieves historical macro indicator data (inflation, unemployment, interest rates) with trend analysis

## Memory Reads
None (this is a Phase 1 data collection agent with no upstream dependencies)

## Memory Writes
After successful data retrieval, store:
```bash
# (removed: claude-flow memory store -k "market/data/{ticker}/news" --value '{"ticker":"...","articles":[...],"overall_sentiment":...}' --namespace default)
# (removed: claude-flow memory store -k "market/data/{ticker}/macro_calendar" --value '{"days":30,"events":[...],"high_impact_count":...}' --namespace default)
# (removed: claude-flow memory store -k "market/data/{ticker}/macro_history" --value '{"indicator":"...","symbol":"SPY","data_points":[...],"current_value":...,"trend":"..."}' --namespace default)
```

## Prompt Template
```
## YOUR TASK
Fetch news articles, macroeconomic calendar, and macro indicator history for ticker {ticker}. Use MCP tools to retrieve sentiment data and market context. Store results in memory for sentiment analyzer and thesis generator.

## WORKFLOW CONTEXT
Agent #3 of 12 | Phase 1: Data Collection (Parallel) | Previous: None | Next: Sentiment analyzer, Thesis generator

## MEMORY RETRIEVAL
None required (first agent in pipeline)

## MEMORY STORAGE (For Next Agents)
1. For Sentiment Analyzer: key "market/data/{ticker}/news" - News articles with sentiment scores and summaries
2. For Thesis Generator: key "market/data/{ticker}/macro_calendar" - Upcoming macro events with impact levels
3. For Thesis Generator: key "market/data/{ticker}/macro_history" - Historical macro data with trend analysis

## STEPS
1. Call `mcp__market-terminal__get_news({ticker}, limit=20)` to retrieve recent news
2. Call `mcp__market-terminal__get_macro_calendar(days=30)` to retrieve upcoming macro events
3. Call `mcp__market-terminal__get_macro_history(indicator="CPI", symbol="SPY")` to retrieve inflation data
4. Validate data completeness (ensure articles array is not empty, events are present)
5. Store news data to memory key "market/data/{ticker}/news"
6. Store macro calendar to memory key "market/data/{ticker}/macro_calendar"
7. Store macro history to memory key "market/data/{ticker}/macro_history"
8. Verify storage: `mcp__memorygraph__recall_memories with query "market/data/{ticker}/news" --namespace default`

## SUCCESS CRITERIA
- News data retrieved with at least 10 articles
- Macro calendar retrieved with upcoming events
- Macro history retrieved with valid data points
- All three datasets stored in memory and verified
- Error handling in place for missing or stale data
```

## Output Schema
```typescript
interface NewsData {
  ticker: string;
  articles: Array<{
    title: string;
    source: string;
    date: string;
    sentiment: number; // -1.0 to 1.0
    summary: string;
  }>;
  overall_sentiment: number; // -1.0 to 1.0
  error?: string;
}

interface MacroCalendarData {
  days: number;
  events: Array<{
    date: string;
    event: string;
    impact: "high" | "medium" | "low";
    previous: string;
    forecast: string;
  }>;
  high_impact_count: number;
  error?: string;
}

interface MacroHistoryData {
  indicator: string;
  symbol: string;
  data_points: Array<{
    date: string;
    value: number;
  }>;
  current_value: number;
  trend: "rising" | "falling" | "stable";
  error?: string;
}
```

## Data Source Priority

Use the first available data source. Fall back to the next if unavailable or erroring.

1. **MCP Market Terminal** (preferred): `mcp__market-terminal__get_news(symbol, limit=20)`, `mcp__market-terminal__get_macro_calendar(days=30)`, `mcp__market-terminal__get_macro_history(indicator, symbol="SPY")`
2. **Perplexity Search** (secondary): Use `mcp__perplexity__perplexity_search` with queries:
   - News: `"{ticker} stock news last 30 days earnings analyst upgrades downgrades"`
   - Macro calendar: `"macroeconomic calendar upcoming events FOMC jobs GDP CPI next 30 days"`
   - Macro history: `"US CPI inflation rate unemployment rate interest rate trend past 12 months"`
3. **WebSearch** (last resort -- only if perplexity is out of credits): Use `WebSearch` + `WebFetch` with:
   - News: `"{ticker} news site:cnbc.com"` or `"{ticker} stock news site:reuters.com"`
   - Macro calendar: `"economic calendar site:investing.com"` or `"economic calendar site:forexfactory.com"`
   - Macro history: `"CPI data site:tradingeconomics.com"` or `"inflation rate site:bls.gov"`

### Detection Logic
- Try MCP market-terminal tools first. If tool not found or returns error, try perplexity.
- If perplexity returns credit/rate limit error, fall back to WebSearch.
- Always prefer structured data over unstructured when available.
- Parse web results into the NewsData/MacroCalendarData/MacroHistoryData schemas as closely as possible.

## Error Handling
- If `get_news` fails: Store error in news data object with `error` field, continue to macro calendar fetch
- If `get_macro_calendar` fails: Store error in macro calendar object with `error` field, continue to macro history fetch
- If `get_macro_history` fails: Store error in macro history object with `error` field
- If all fail: Store error objects in all three memory keys and flag for manual review
- Missing data: Log warning but continue pipeline with partial data (sentiment analyzer may still run with available news)
- No news articles: Store empty array but continue (sentiment will be neutral)
- Stale news: Log warning if all articles are older than 7 days but continue
