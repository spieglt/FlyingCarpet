package core

import (
	"crypto/cipher"
	"crypto/rand"
	"io"
)

func encrypt(chunk []byte, aesgcm cipher.AEAD) ([]byte, error) {

	nonce := make([]byte, 12)
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		panic(err.Error())
	}

	ciphertext := aesgcm.Seal(nil, nonce, chunk, nil)

	return append(nonce, ciphertext...), nil
}

func decrypt(chunk []byte, aesgcm cipher.AEAD) ([]byte, error) {
	nonce := chunk[:12]

	plaintext, err := aesgcm.Open(nil, nonce, chunk[12:], nil)
	if err != nil {
		panic(err.Error())
	}

	return plaintext, nil
}
