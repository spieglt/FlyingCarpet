package core

import (
	"crypto/rand"
	"errors"
	"io"

	"golang.org/x/crypto/nacl/secretbox"
)

func encrypt(chunk []byte, passphrase []byte) ([]byte, error) {
	var err error
	var key [32]byte
	copy(key[:], passphrase)

	var nonce [24]byte
	_, err = io.ReadFull(rand.Reader, nonce[:])
	if err != nil {
		return nil, err
	}

	return secretbox.Seal(nonce[:], chunk, &nonce, &key), nil
}

func decrypt(chunk []byte, passphrase []byte) ([]byte, error) {
	var key [32]byte
	copy(key[:], passphrase)

	var decryptNonce [24]byte
	copy(decryptNonce[:], chunk[:24])

	decrypted, ok := secretbox.Open(nil, chunk[24:], &decryptNonce, &key)
	if !ok {
		return []byte{}, errors.New("error decrypting")
	}
	return decrypted, nil
}
