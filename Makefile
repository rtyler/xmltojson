
README.md: README.adoc
	asciidoc -b docbook README.adoc && pandoc -f docbook -t markdown_mmd -o README.md README.xml

clean:
	rm -f README.md README.xml
