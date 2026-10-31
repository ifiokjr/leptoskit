# Sign in with Apple

Lets people sign in with their Apple Account. The App Store expects an app that offers other third-party sign-in on iOS to offer Sign in with Apple too (App Review guideline 4.8), and `leptoskit auth doctor` reminds you. Last checked against Apple's documentation on 30 October 2026.

## Set it up

1. Run `leptoskit auth provider add apple`. It adds:

   ```toml
   [auth.providers.apple]
   client_id_env = "APPLE_SERVICES_ID"
   team_id_env = "APPLE_TEAM_ID"
   key_id_env = "APPLE_KEY_ID"
   private_key_env = "APPLE_PRIVATE_KEY"
   ```

2. In [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources), create a Services ID, turn on Sign in with Apple for it, and add your domain and the return URL `https://myapp.com/auth/callback/apple`. Apple doesn't accept `localhost`; test with a tunnel that gives you an `https` address.
3. Create a key with Sign in with Apple enabled, and download its `.p8` file. Apple lets you download it once.
4. Set `APPLE_SERVICES_ID` (the Services ID), `APPLE_TEAM_ID`, `APPLE_KEY_ID`, and `APPLE_PRIVATE_KEY` (the `.p8` file's contents) in your server's environment.
5. Turn on the `auth-providers` feature, and run `leptoskit auth doctor`.

## What sign-in gets

A stable ID for the person and their email address, which Apple vouches for. It may be a private relay address that forwards to them, such as `x7…@privaterelay.appleid.com`. Apple sends the person's name only the first time they sign in, so leptoskit stores it then.

Apple doesn't take a fixed client secret: leptoskit signs a short one for each sign-in with your `.p8` key, so there's nothing to rotate. Apple sends people back by posting a form from its own site; leptoskit answers that post with a page that posts it again from your site, where the sign-in's cookie is sent, and only then signs anyone in.

## Platforms

Sign in with Apple works on the web. Native apps, including iOS, answer `unsupported` until native Sign in with Apple and the system-browser plugin arrive.
