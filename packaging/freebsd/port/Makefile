PORTNAME=	host-identity
DISTVERSION=	@@VERSION@@
CATEGORIES=	sysutils
MASTER_SITES=	https://github.com/dekobon/host-identity/releases/download/v${DISTVERSION}/
DISTNAME=	host-identity-${DISTVERSION}-x86_64-unknown-freebsd
EXTRACT_SUFX=	.tar.gz

MAINTAINER=	e.zupancic@f5.com
COMMENT=	Stable host UUID across platforms, clouds, and Kubernetes
WWW=		https://github.com/dekobon/host-identity

LICENSE=	APACHE20 MIT
LICENSE_COMB=	dual

NO_BUILD=	yes
ONLY_FOR_ARCHS=	amd64

PLIST_FILES=	bin/host-identity \
		share/man/man1/host-identity.1.gz \
		share/man/man1/host-identity-audit.1.gz \
		share/man/man1/host-identity-resolve.1.gz \
		share/man/man1/host-identity-sources.1.gz

do-install:
	${INSTALL_PROGRAM} ${WRKSRC}/host-identity ${STAGEDIR}${PREFIX}/bin/host-identity
.for m in host-identity host-identity-audit host-identity-resolve host-identity-sources
	${INSTALL_MAN} ${WRKSRC}/man/${m}.1 ${STAGEDIR}${PREFIX}/share/man/man1/${m}.1
.endfor

.include <bsd.port.mk>
