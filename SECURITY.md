# Security Policy

## Reporting a Vulnerability

If you believe you have found a security vulnerability in this crate,
please report it privately. **Do not open a public GitHub issue.**

Please report vulnerabilities via one of the following:

- **GitHub Security Advisories** (preferred): Use the
  ["Report a vulnerability"](https://github.com/dekobon/host-identity/security/advisories/new)
  button on this repository.
- **Email**: elijah@zupancic.name
- **PGP**:
```
-----BEGIN PGP PUBLIC KEY BLOCK-----

mQMuBGB0c6URCACZ2fznACJ+yxh0/zs/3GtELyfopjepW2B4Oevd3Jb3ULU/r1wE
ylyixjzwR0zRigrbjAAmjIbbhcX1/dGO2lcX1+aRtm6hgyL4cqf19/sF+fPJmqSI
1/CMAFu8Ku+xo9GAYLk74My2TF37Wm7eNCH38bwMMBPdiLsA4dd+76VZON0WJ72C
r8z3XAwle9vNLPdnmKGiRtwqq0grTFVNJBZVofxzqB89AXgRD6OOuGmqIg/BFby5
3yF6q1c5ii2yZDQVjyG2ZRkTVxeNMBKIw8aIBtYFT5sAYGmJ46/37b7N7XMlmeuz
TSuox7al6lGvkfC784arE2hAJeFZVUO/kBrvAQCp8YI/qDPV9Os9RcUMqMc/pS6X
fbvdgEb8SF8gT3OgfQf/UIgmxlYF942EjkZfM8/A0w+JoadoeXOv6BjTVU1e1sCQ
Yriv4OvRL/zSKuv5I+t4PpI2aooofbEa2bIBJ1QpZ047IRA0HkGpe1jP9EFstHJe
Oo65oeK7i2PiSM6VPX4MEou5xzT/1i/X+diK0tr4xjlqPvxjS1RNA0kY1mKmWlJE
8vgteUxdQrrndqFMigBcvOtJsbUd2AZa1mZaPwgo47IaulWHeJK8J3OZ8o/2hURo
g19iKFf99/ujzFDSJEwjqAX4+dhXaTpmfvO8fwRBFNpyoExBsBVujQAZ3qyt7e8J
5fUBG7A8OwMDlLM8uoJoFcyWeEmCQRVSglle5JfXLAgAl1soy663ZSE88izpGFWJ
fZ7acyV4kesnFrBj8z5K+Cz8/729az+7tE7LYutukWdPuKRSiucsz1NwH6py3naF
t+dFhTMYuIke5OdL0ThY4RH8N76O9v569BYVzrOZLrCT//zdzy/WI6lnMHBXzwZS
0UA+9myNPeScioRJyXW7FRVTBKDL5VjEShlJRvuW2COhJs6sHLQWqYsaOmsXRgi6
0IOuGou+MPwZYPfVeAbMLUhBAbmvQzHZ3KcVgdlGN9yRHXILBM8miDOxBWLm3vwa
KPUlrYobSiZn2uJ8Noq6aRJ4ugxHZDN4C7GY8kE+IppgRMjVgVhewnUGnG/l7qQy
V7Q7RWxpamFoIFp1cGFuY2ljIChnaXRodWIgc2lnbmluZyBrZXkpIDxlbGlqYWhA
enVwYW5jaWMubmFtZT6IkAQTEQgAOBYhBKv+nbWN+dVrEqlPyaiQz1dGsV5SBQJg
dHOlAhsjBQsJCAcCBhUKCQgLAgQWAgMBAh4BAheAAAoJEKiQz1dGsV5S8CwBAKSX
HvjcdYIi3RY4k/sZPiBpg+8PSWRItjGEwjykYbzCAQCJEj71mPVeo+4tn3q2eCAl
bObxoqkdiJLTO28DmfRMxbhSBGB0c6UTCCqGSM49AwEHAgMES3Plz6xdPSUq6eWr
1vaFyH3aWQDvDbnC4QhFIA2whPZA6/pOj1iKkE4/ymoZY2hV+wEuve+Gcarfm4Pq
ZiDmkIjvBBgRCAAgFiEEq/6dtY351WsSqU/JqJDPV0axXlIFAmB0c6UCGyIAgQkQ
qJDPV0axXlJ2IAQZEwgAHRYhBLnb4FuwnerbBakrtb4WwgCefmELBQJgdHOlAAoJ
EL4WwgCefmELl+kA/0EPXpcbFjLDDUSBxDGU2mEjgXte8rWR+qd/WUnGpdXtAP4g
hSsynVx4g7kSULk6hIkjnxO7IBDm/8yvtL2rXUZWEJa2AQCnFvO6azorQNgTgLfp
SO4hc9BFJyAmnI2nkGnpMKCEWQD/R+Wct/VrjzhbTbqb+9zK8B91sUBz90EZGMsv
nxVZujA=
=9LHv
-----END PGP PUBLIC KEY BLOCK-----
```


Please include the following in your report:

- A description of the vulnerability and its impact
- Steps to reproduce, or a proof-of-concept
- The affected version(s)
- Any suggested mitigation, if known

## Response Expectations

- We will acknowledge receipt within **3 business days**.
- We aim to provide an initial assessment within **7 business days**.
- We will keep you informed of progress toward a fix and coordinate
  a disclosure timeline with you.
- Typical time-to-fix for confirmed vulnerabilities is **30–90 days**
  depending on severity and complexity.

## Disclosure Policy

We follow **coordinated disclosure**. Once a fix is available:

1. We will publish a patched release on crates.io.
2. We will publish a [RustSec advisory](https://rustsec.org/) with a
   CVE identifier where appropriate.
3. We will credit the reporter in the advisory unless they prefer
   to remain anonymous.

We ask that reporters give us a reasonable window (typically 90 days)
to release a fix before public disclosure.

## Scope

This policy covers vulnerabilities in the code of this crate itself.
Vulnerabilities in dependencies should be reported to the respective
upstream projects; we will update our dependency requirements promptly
once upstream fixes are available.

## Safe Harbor

We consider security research conducted in good faith under this policy
to be authorized. We will not pursue legal action against researchers
who:

- Make a good-faith effort to avoid privacy violations, data destruction,
  or service disruption
- Report vulnerabilities promptly
- Do not exploit the vulnerability beyond what is necessary to demonstrate it
- Give us reasonable time to respond before public disclosure
