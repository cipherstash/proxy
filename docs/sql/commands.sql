
-- INSERT users
INSERT INTO users (encrypted_email, encrypted_dob, encrypted_salary) VALUES ('alice@cipherstash.com', '1970-01-01', '100');
INSERT INTO users (encrypted_email, encrypted_dob, encrypted_salary) VALUES ('bob@cipherstash.com', '1991-03-06', '10');
INSERT INTO users (encrypted_email, encrypted_dob, encrypted_salary) VALUES ('carol@cipherstash.com', '2005-12-30', '1000');

-- SELECT user by email
SELECT encrypted_dob FROM users WHERE encrypted_email = 'alice@cipherstash.com';

-- UPDATE user by email
UPDATE users SET encrypted_dob = '1978-02-01' WHERE encrypted_email = 'alice@cipherstash.com';


-- Comparing salary
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users WHERE encrypted_salary <= 100;

-- Comparing dob
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users WHERE encrypted_dob > '2000-01-01' ;

-- Searching for email
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users WHERE encrypted_email LIKE 'alice';