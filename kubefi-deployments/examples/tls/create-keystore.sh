#!/usr/bin/env bash

# below generated keystores are only intended for development purposes

password=nerijni3983sdD
NIFI_NODE_TLS_CN=""
name=nifi

if [[ -z "$1" ]]
then
      echo "Usage: <NIFI_NODE_TLS_CN>"
      exit 1
else
      NIFI_NODE_TLS_CN=$1
      echo "using $NIFI_NODE_TLS_CN as NIFI_NODE_TLS_CN"
fi

cn="$NIFI_NODE_TLS_CN"

rm *.csr *.crt *.srl *.jks *.key *_creds

CA_SUBJECT="/CN=kubefi.novakov-alexey.github.io/OU=TEST"
NIFI_NODE_OU="OU=TEST"
SAN=$2

openssl req -new -x509 -keyout snakeoil-ca-1.key -out snakeoil-ca-1.crt -days 365 -subj "$CA_SUBJECT" -passin pass:$password -passout pass:$password

keytool -genkeypair -noprompt \
             -alias $name \
             -ext SAN=dns:$SAN \
             -dname "CN=$cn, $NIFI_NODE_OU" \
             -keystore keystore.jks \
             -keyalg RSA \
             -storepass $password \
             -keypass $password

keytool -keystore keystore.jks -alias $name -certreq -file $name.csr -storepass $password -keypass $password -ext SAN=dns:$SAN

openssl x509 -req -new -CA snakeoil-ca-1.crt -CAkey snakeoil-ca-1.key -in $name.csr -out $name-ca1-signed.crt -days 9999 \
  -CAcreateserial -passin pass:$password \
  -extensions san \
  -config san.cnf

keytool -keystore keystore.jks -alias CARoot -import -file snakeoil-ca-1.crt -storepass $password \
  -keypass $password -ext SAN=dns:$SAN

keytool -keystore keystore.jks -alias $name -import -file $name-ca1-signed.crt -storepass $password \
  -keypass $password -ext SAN=dns:$SAN

keytool -keystore truststore.jks -alias CARoot -import -file snakeoil-ca-1.crt -storepass $password \
  -keypass $password -ext SAN=dns:$SAN

echo "$password" > keystore_creds

echo "$password" > truststore_creds