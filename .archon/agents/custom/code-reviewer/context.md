# Code Review Domain Context

## Security Checklist (OWASP Top 10 for Code)
1. **Injection**: SQL injection, command injection, template injection
2. **Broken Auth**: Hardcoded credentials, weak token generation
3. **Sensitive Data**: Secrets in code, unencrypted data at rest
4. **XXE/Deserialization**: Unsafe YAML/pickle/eval usage
5. **Access Control**: Missing authorization checks, IDOR
6. **Misconfiguration**: Debug mode in production, overly permissive CORS
7. **XSS**: Unescaped user input in templates/responses
8. **Insecure Dependencies**: Known CVEs in imported packages
9. **Logging**: Sensitive data in logs, missing audit trails
10. **SSRF**: Unvalidated URLs in HTTP requests

## Python-Specific Patterns
- Use `isinstance(x, bool)` before `isinstance(x, int)` (bool is subclass of int)
- Check `math.isnan()`/`math.isinf()` for float fields before arithmetic
- Prefer `pathlib.Path` over `os.path` for file operations
- Use `secrets` module (not `random`) for security-sensitive values
- Type hints on all public functions

## TypeScript-Specific Patterns
- Use strict TypeScript (`strict: true` in tsconfig)
- Prefer `unknown` over `any` for untyped external data
- Use `readonly` for immutable properties
- Validate API response shapes at runtime (zod, io-ts)
- Avoid `as` type assertions without validation
