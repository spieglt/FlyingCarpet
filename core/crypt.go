package core

import (
	"crypto/rand"
	"errors"
	"io"

	"golang.org/x/crypto/bcrypt"
	"golang.org/x/crypto/nacl/secretbox"
)

func encrypt(chunk []byte, passphrase string) ([]byte, error) {

	hashedPassphrase, err := bcrypt.GenerateFromPassword([]byte(passphrase), 0)
	if err != nil {
		return nil, err
	}

	var key [32]byte
	copy(key[:], hashedPassphrase)

	var nonce [24]byte
	_, err = io.ReadFull(rand.Reader, nonce[:])
	if err != nil {
		return nil, err
	}

	return secretbox.Seal(nonce[:], chunk, &nonce, &key), nil
}

func decrypt(chunk []byte, passphrase string) ([]byte, error) {

	hashedPassphrase, err := bcrypt.GenerateFromPassword([]byte(passphrase), 0)
	if err != nil {
		return nil, err
	}

	var key [32]byte
	copy(key[:], hashedPassphrase)

	var decryptNonce [24]byte
	copy(decryptNonce[:], chunk[:24])

	decrypted, ok := secretbox.Open(nil, chunk[24:], &decryptNonce, &key)
	if !ok {
		return []byte{}, errors.New("error decrypting")
	}
	return decrypted, nil
}
