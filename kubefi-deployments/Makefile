.ONESHELL:
.SHELL := /bin/bash

DEV_ENV=make.dev.env
include $(DEV_ENV)
export

args = `arg="$(filter-out $@,$(MAKECMDGOALS))" && echo $${arg:-${1}}`

OPERATOR_NAMESPACE=test
PATH_TO_CONFIGS=conf/
KUBEFI_VER=./read-version.sh
IMAGE_REGISTRY=alexeyn

show-version:
	cargo pkgid

deploy-configs:
	kubectl create configmap kubefi-configs \
      --from-file=kubefi.conf=$(PATH_TO_CONFIGS)kubefi.conf \
      --from-file=nifi.conf=$(PATH_TO_CONFIGS)nifi.conf \
      --from-file=schema.json=$(PATH_TO_CONFIGS)schema.json \
      -n $(OPERATOR_NAMESPACE)

deploy-kubefi: deploy-configs
	sed -e "s:{{NAMESPACE}}:$(OPERATOR_NAMESPACE):g" manifests/rbac.yaml | kubectl create -n $(OPERATOR_NAMESPACE) -f -
	sed -e "s:{{INGRESS_HOST}}:$(INGRESS_HOST):g" -e "s:{{KUBEFI_VERSION}}:$(KUBEFI_VER):g" \
		manifests/kubefi-deployments-operator.yaml | kubectl create -n $(OPERATOR_NAMESPACE) -f -

undeploy-kubefi:
	kubectl delete cm kubefi-configs -n $(OPERATOR_NAMESPACE)
	sed -e "s:{{NAMESPACE}}:$(OPERATOR_NAMESPACE):g" manifests/rbac.yaml | kubectl delete -n $(OPERATOR_NAMESPACE) -f -
	sed -e "s:{{INGRESS_HOST}}:$(INGRESS_HOST):g" -e "s:{{KUBEFI_VERSION}}:$(KUBEFI_VER):g" \
		manifests/kubefi-deployments-operator.yaml | kubectl delete -n $(OPERATOR_NAMESPACE) -f -

build-image:
	sh build-image.sh $(KUBEFI_VER)

push-image: build-image
	docker tag kubefi-deployments-operator:$(KUBEFI_VER) $(IMAGE_REGISTRY)/kubefi-deployments-operator:$(KUBEFI_VER)
	docker push $(IMAGE_REGISTRY)/kubefi-deployments-operator:$(KUBEFI_VER)