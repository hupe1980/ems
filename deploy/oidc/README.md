# Browser sign-in

Publish `/portal` and `/desk` behind one HTTPS origin. OAuth2 Proxy handles sign-in with your OpenID Connect provider and keeps the session in a secure, HTTP-only cookie. NGINX maps the verified subject to the existing household or tenant-scoped API credential. The browser receives neither credential.

1. Register a confidential web client with one issuer and the exact callback `https://ems.example.org/oauth2/callback`. Enable Authorization Code with PKCE (S256). Use your provider’s assignment and MFA policies.
2. Send the entire HTTPS origin, including `/oauth2/`, to OAuth2 Proxy on loopback port 4180. Start it with `--config=oauth2-proxy.cfg`. Set `OAUTH2_PROXY_OIDC_ISSUER_URL`, `OAUTH2_PROXY_CLIENT_ID`, `OAUTH2_PROXY_CLIENT_SECRET`, `OAUTH2_PROXY_REDIRECT_URL` and `OAUTH2_PROXY_COOKIE_SECRET`. Generate the cookie secret with `openssl rand -base64 32`. Use the same secret on all proxy replicas.
3. Include `ems.conf` inside NGINX’s `http` context. The two map files contain one entry per identity grant. Use the issuer’s immutable `sub` value, escaped for a case-sensitive anchored regex. For example, a subject of `home-42` has an entry of the following form in `/run/secrets/ems-household.map`.

   ```nginx
   ~^home-42$ "Bearer REPLACE_WITH_SITE_SCOPED_SECRET";
   ```

   The secret must match that household’s `obsd.customer_tokens` entry. The operator map uses a credential from `fleetd.operators` scoped to the operator’s tenant. An empty file grants nobody that role. Mount both files as secrets and reload NGINX after grant changes. Removing a grant takes effect on the next request after reload.
4. Keep NGINX’s upstream listener reachable only by OAuth2 Proxy. Keep the daemon listeners private. The subject header is trustworthy only on that hop. The example assumes these processes share a trusted loopback network. The public listener must never route around OAuth2 Proxy.

Visit `/portal` as a household member or `/desk` as an operator. A valid identity with no matching grant receives 403. Sign out ends the EMS session. The identity provider controls its own SSO session.

Run `uv run xtask/oidc_browser.py` from the workspace root to rehearse sign-in and sign-out with disposable Dex, OAuth2 Proxy and NGINX containers. The trial uses Docker, Chromium and `htpasswd`.

[OAuth2 Proxy OIDC configuration](https://oauth2-proxy.github.io/oauth2-proxy/configuration/providers/openid_connect/)
