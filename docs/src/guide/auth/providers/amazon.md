# Amazon

Lets people sign in with their Amazon account, through Login with Amazon. Last checked against Amazon's documentation on 31 October 2026.

## Set it up

1. Run `leptoskit auth provider add amazon`. It adds:

   ```toml
   [auth.providers.amazon]
   client_id_env = "AMAZON_CLIENT_ID"
   client_secret_env = "AMAZON_CLIENT_SECRET"
   ```

2. In the [Login with Amazon console](https://developer.amazon.com/loginwithamazon/console/site/lwa/overview.html), create a security profile and open its Web Settings.
3. Add the Allowed Return URLs `https://myapp.com/auth/callback/amazon`, and `http://localhost:3000/auth/callback/amazon` for development.
4. Set `AMAZON_CLIENT_ID` and `AMAZON_CLIENT_SECRET`.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

The person's Amazon user ID for your app, their name, and their email address. leptoskit asks for the `profile` scope.

Amazon doesn't say whether it checked the address, so leptoskit keeps it on the connected account for display, but never stores it as the account's own or uses it to find another account. Accounts made with Amazon start without an email address.

## Platforms

It works on the web, and in iOS, macOS, and Android apps through the system browser, with the same redirect URIs: the browser comes back to your site, which sends it on to the app. See [Sign-in](../../23-sign-in.md#sign-in-with-other-sites) for each platform.
