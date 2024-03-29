.ONESHELL:
.SHELL := /bin/bash

DEV_ENV=make.dev.env
include $(DEV_ENV)
export

args = `arg="$(filter-out $@,$(MAKECMDGOALS))" && echo $${arg:-${1}}`

OPERATOR_NAMESPACE=test
EXAMPLE_NAMESPACE=$(OPERATOR_NAMESPACE)
PATH_TO_CONFIGS=conf/
IMAGE_REGISTRY=alexeyn
KUBEFI_VER=0.1.2

show-version:
	cargo pkgid

deploy-configs:
	kubectl create configmap kubefi-configs \
      --from-file=kubefi.conf=$(PATH_TO_CONFIGS)kubefi.conf \
      --from-file=nifi.conf=$(PATH_TO_CONFIGS)nifi.conf \
      --from-file=schema.json=$(PATH_TO_CONFIGS)schema.json \
      -n $(OPERATOR_NAMESPACE)
install: deploy-configs
	sed -e "s:{{NAMESPACE}}:$(OPERATOR_NAMESPACE):g" manifests/rbac.yaml | kubectl create -n $(OPERATOR_NAMESPACE) -f -
	sed -e "s:{{INGRESS_HOST}}:$(INGRESS_HOST):g" -e "s:{{KUBEFI_VERSION}}:$(KUBEFI_VER):g" \
		manifests/kubefi-deployments-operator.yaml | kubectl create -n $(OPERATOR_NAMESPACE) -f -
uninstall:
	kubectl delete cm kubefi-configs -n $(OPERATOR_NAMESPACE)
	sed -e "s:{{NAMESPACE}}:$(OPERATOR_NAMESPACE):g" manifests/rbac.yaml | kubectl delete -n $(OPERATOR_NAMESPACE) -f -
	sed -e "s:{{INGRESS_HOST}}:$(INGRESS_HOST):g" -e "s:{{KUBEFI_VERSION}}:$(KUBEFI_VER):g" \
		manifests/kubefi-deployments-operator.yaml | kubectl delete -n $(OPERATOR_NAMESPACE) -f -
run:
	REPLACE_EXISTING_CRD=true cargo run
build-image:
	# build musl binary via docker
	rm -r target || exit 0
	docker run --rm -it  -v "$$(pwd)":/home/rust/src ekidd/rust-musl-builder:stable cargo build --release

	# prepare content for the app image
	mkdir target/docker
	cp -r templates/ target/docker/templates
	cp docker/Dockerfile target/docker
	cp target/x86_64-unknown-linux-musl/release/kubefi-deployments target/docker/
	mkdir -p target/docker/conf/
	cp -r conf/ target/docker/conf

	# build the app image
	docker build -t kubefi-deployments-operator:$(KUBEFI_VER) -f target/docker/Dockerfile target/docker/

push-image:
	docker tag kubefi-deployments-operator:$(KUBEFI_VER) $(IMAGE_REGISTRY)/kubefi-deployments-operator:$(KUBEFI_VER)
	docker push $(IMAGE_REGISTRY)/kubefi-deployments-operator:$(KUBEFI_VER)

add-ingress-repo:
	helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx	
	helm repo update
deploy-ingress-controller:
	helm install ingress-nginx ingress-nginx/ingress-nginx \
	--namespace default \
	--set controller.replicaCount=1 \
	--set controller.nodeSelector."beta\.kubernetes\.io/os"=linux \
    --set defaultBackend.nodeSelector."beta\.kubernetes\.io/os"=linux \
    --set-string controller.podAnnotations.enable-ssl-passthrough=true \
    --set controller.extraArgs.enable-ssl-passthrough=""
undeploy-ingress-controller:
	helm delete ingress-nginx --namespace default

create-tls-jks-secret:
	kubectl delete secret nifi-tls-jks -n $(EXAMPLE_NAMESPACE) || exit 0
	kubectl create secret generic nifi-tls-jks \
    	--from-file=./examples/tls/keystore.jks \
    	--from-file=./examples/tls/truststore.jks \
    	-n $(EXAMPLE_NAMESPACE)
create-tls-password-secret:
	kubectl create secret generic nifi-tls-pwd \
    	--from-file=./examples/tls/keyPasswd \
    	--from-file=./examples/tls/keystorePasswd \
    	--from-file=./examples/tls/truststorePasswd \
    	-n $(EXAMPLE_NAMESPACE)
create-tls-secrets: create-tls-jks-secret create-tls-password-secret

create-logback-conf:
	kubectl create -f examples/custom-logback-cm.yaml -n $(EXAMPLE_NAMESPACE)

deploy-example:
	kubectl create -f examples/my-nifi-deployment.yaml -n $(EXAMPLE_NAMESPACE)
undeploy-example:
	kubectl delete -f examples/my-nifi-deployment.yaml -n $(EXAMPLE_NAMESPACE)

deploy-ldap:
	kubectl create -f ldap/ldap-deployment.yaml -n $(EXAMPLE_NAMESPACE)
	kubectl create -f ldap/ldap-service.yaml -n $(EXAMPLE_NAMESPACE)
undeploy-ldap:
	kubectl delete -f ldap/ldap-deployment.yaml -n $(EXAMPLE_NAMESPACE)
	kubectl delete -f ldap/ldap-service.yaml -n $(EXAMPLE_NAMESPACE)