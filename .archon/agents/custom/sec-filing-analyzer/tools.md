# Tool Instructions for SEC Filing Analysis

## Primary Tools
- **Read**: Use to read filing documents (text files, HTML) if stored locally
- **Bash**: Use `curl` to fetch filings from EDGAR if needed
  - EDGAR full-text search: `https://efts.sec.gov/LATEST/search-index?q="{company}" AND "10-K"&dateRange=custom&startdt={start}&enddt={end}`
  - Company filings: `https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK={cik}&type=10-K&dateb=&owner=include&count=10`
- **Grep**: Use to search filing text for specific terms (e.g., "revenue recognition", "restatement", "material weakness")

## Analysis Workflow
1. Locate the filing (local file or EDGAR URL)
2. Search for revenue recognition policy section
3. Search for risk factors mentioning accounting/revenue
4. Extract key financial figures from financial statements
5. Cross-reference MD&A narrative with financial data
6. Compile findings into structured output
