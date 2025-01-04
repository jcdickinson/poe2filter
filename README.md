# POE 2 Filter

Automatic filter updater for POE2, for Linux.

## Usage

In the steam game properties change the command to:

```
poe2filter -- %command%
```

You can use it before or after your other wrappers, it shouldn't really matter.

## Debug

This will perform detailed logging. You will typically have to start Steam from the terminal in order to see this.

```
POE2FILTER_LOG=trace poe2filter -- %command%
```

## Cachix

If you're using this on Nix:

```
https://jcdickinson.cachix.org
jcdickinson.cachix.org-1:GZBOGJF64N2yc8z/iAlApnNGgGQv1ApmuMz7xaU5dnY=
```
