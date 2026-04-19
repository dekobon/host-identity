PORTNAME=	hostid
DISTVERSION=	@@VERSION@@
CATEGORIES=	sysutils
MASTER_SITES=	https://github.com/dekobon/host-identity/releases/download/v${DISTVERSION}/
DISTNAME=	hostid-${DISTVERSION}-x86_64-unknown-freebsd
EXTRACT_SUFX=	.tar.gz

MAINTAINER=	e.zupancic@f5.com
COMMENT=	Stable host UUID across platforms, clouds, and Kubernetes
WWW=		https://github.com/dekobon/host-identity

LICENSE=	APACHE20 MIT
LICENSE_COMB=	dual

NO_BUILD=	yes
ONLY_FOR_ARCHS=	amd64

PLIST_FILES=	bin/hostid \
		share/man/man1/hostid.1.gz \
		share/man/man1/hostid-audit.1.gz \
		share/man/man1/hostid-resolve.1.gz \
		share/man/man1/hostid-sources.1.gz

do-install:
	${INSTALL_PROGRAM} ${WRKSRC}/hostid ${STAGEDIR}${PREFIX}/bin/hostid
.for m in hostid hostid-audit hostid-resolve hostid-sources
	${INSTALL_MAN} ${WRKSRC}/man/${m}.1 ${STAGEDIR}${PREFIX}/share/man/man1/${m}.1
.endfor

.include <bsd.port.mk>
